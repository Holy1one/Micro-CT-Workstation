//! Single camera/COM owner. Callers hold only cloneable Send message proxies.
//! Stop invalidates a generation immediately; blocking COM cannot delay OFF or
//! state reads and its late return cannot publish, capture or reopen Live View.

use super::capture::capture_with_policy;
use super::live_view::*;
use super::nikon::*;
use super::properties::{refresh_property_controls,clear_live_view_controls};
use super::ptp::*;
use super::types::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
/// Budget for a Live View control transaction (controls readback / configure).
/// These run on the single camera worker, so a Live View transaction that hangs
/// blocks every queued command behind it — including the Stop path's end-of-Live-
/// View, which is issued with a short bound of its own. Hardware reproduced
/// exactly that: a `camera_live_view_read` still in flight made a following Stop
/// time out in `EndLiveView 0x9202` and end `cleanup_unconfirmed`. Keeping this
/// budget well under the Stop pre-emption budget is what prevents that.
const LV_CONTROL_TIMEOUT: Duration = Duration::from_secs(2);
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(75);
const LV_FRAME_INTERVAL: Duration = Duration::from_millis(100);

enum Request {
    Discover,
    Connect(String),
    Disconnect,
    Read,
    Apply(CameraProperties),
    PrepareHost,
    Capture {
        path: PathBuf,
        operation_id: String,
        events: Option<mpsc::Sender<CameraCaptureEvent>>,
        scan_policy: bool,
    },
    LvStart(u64),
    LvStop,
    LvRead(String,u64),
    LvConfigure(String,CameraProperties,u64),
    Af(Option<String>,u64),
    Focus(i32,String,u64),
}
enum Reply {
    Health(CameraHealth),
    Devices(Vec<CameraIdentity>),
    Properties(BTreeMap<String, CameraProperty>),
    Applied(PropertyApplyResult),
    Captured(CapturedImage),
    Done,
}
struct Message {
    request: Request,
    context: CallContext,
    response: mpsc::SyncSender<Result<Reply, CameraError>>,
}
struct Control {
    cached: Mutex<CameraHealth>,
    generation: Arc<AtomicU64>,
    live_generation:AtomicU64,
    live_stop_pending:AtomicBool,
    frames: LatestFrameSlot,
    commit_gate: Mutex<()>,
    deadline: Mutex<Option<(u64, Instant)>>,
    shutter_target: Mutex<Option<CameraValue>>,
    /// Operator attestation side-channel, exactly like the shutter target: it is
    /// merged into every health snapshot and never mutates the worker's own
    /// cached evidence. Validity against the current session is decided by the
    /// domain layer, so a stale record stays visible but never counts as proof.
    operator_multiple_exposure: Mutex<Option<super::types::OperatorMultipleExposureConfirmation>>,
}
#[derive(Clone)]
pub struct CameraAdapter {
    queue: mpsc::SyncSender<Message>,
    control: Arc<Control>,
}
impl CameraAdapter {
    /// Starts an idle COM owner only. No discovery, connection or hardware I/O.
    pub fn new() -> Result<Self, CameraError> {
        Self::with_factory(super::wpd::create_transport)
    }
    pub fn discover() -> Result<Self, CameraError> {
        Self::new()
    }
    pub(crate) fn with_factory(
        factory: impl FnOnce() -> Result<Box<dyn CameraTransport>, CameraError> + Send + 'static,
    ) -> Result<Self, CameraError> {
        let (queue, receiver) = mpsc::sync_channel(16);
        let (ready, initialized) = mpsc::sync_channel(1);
        let control = Arc::new(Control {
            cached: Mutex::new(CameraHealth {
                backend: "windows_wpd_ptp".into(),
                cleanup_confirmed: true,
                phase: "disconnected".into(),
                ..CameraHealth::default()
            }),
            generation: Arc::new(AtomicU64::new(1)),
            live_generation:AtomicU64::new(1),
            live_stop_pending:AtomicBool::new(false),
            frames: LatestFrameSlot::default(),
            commit_gate: Mutex::new(()),
            deadline: Mutex::new(None),
            shutter_target: Mutex::new(None),
            operator_multiple_exposure: Mutex::new(None),
        });
        let worker_control = control.clone();
        thread::Builder::new()
            .name("camera-com-owner".into())
            .spawn(move || {
                // Factory runs here; !Send COM interfaces never cross the boundary.
                let transport = match factory() {
                    Ok(t) => {
                        let _ = ready.send(Ok(()));
                        t
                    }
                    Err(e) => {
                        let _ = ready.send(Err(e));
                        return;
                    }
                };
                worker(NativeCamera::new(transport), receiver, worker_control);
            })
            .map_err(CameraError::from)?;
        initialized
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| CameraError::Timeout("COM owner initialization".into()))??;
        let watch = Arc::downgrade(&control);
        thread::Builder::new().name("camera-deadline-watchdog".into()).spawn(move || {
            while let Some(control) = watch.upgrade() {
                let expired = control.deadline.lock().unwrap().is_some_and(|(epoch, deadline)| epoch == control.generation.load(Ordering::Acquire) && Instant::now() >= deadline);
                if expired { invalidate(&control, "Camera driver deadline exceeded; session isolated; cleanup unconfirmed"); *control.deadline.lock().unwrap() = None; }
                drop(control); thread::sleep(Duration::from_millis(10));
            }
        }).map_err(CameraError::from)?;
        Ok(Self { queue, control })
    }
    /// Nonblocking with respect to all camera I/O, including a stalled driver.
    pub fn health(&self) -> CameraHealth {
        self.control_snapshot()
    }
    pub fn control_status(&self) -> CameraHealth {
        self.control_snapshot()
    }
    pub fn control_snapshot(&self) -> CameraHealth {
        let mut h = self.control.cached.lock().unwrap().clone();
        if let Some(target) = self.control.shutter_target.lock().unwrap().clone() {
            if let Some(property) = h.properties.get_mut("shutter") {
                property.target = Some(target);
            }
        }
        if h.live_view.active && !self.control.frames.is_fresh() {
            h.live_view.fresh = false;
        }
        h.operator_multiple_exposure = self.control.operator_multiple_exposure.lock().unwrap().clone();
        h
    }
    /// Records an operator attestation for the current session. Returns the
    /// stored record so the caller reports exactly what was accepted.
    pub fn set_operator_multiple_exposure(
        &self,
        record: Option<super::types::OperatorMultipleExposureConfirmation>,
    ) -> Option<super::types::OperatorMultipleExposureConfirmation> {
        let mut slot = self.control.operator_multiple_exposure.lock().unwrap();
        *slot = record;
        slot.clone()
    }
    pub fn operator_multiple_exposure(&self) -> Option<super::types::OperatorMultipleExposureConfirmation> {
        self.control.operator_multiple_exposure.lock().unwrap().clone()
    }
    pub fn pipe_descriptor(&self) -> Option<LiveViewPipeDescriptor> {
        self.control.frames.descriptor()
    }
    /// Acceptance is synchronous and independent of the normal queue/COM call.
    /// No claim of hardware cancellation or cleanup is made at this boundary.
    pub fn request_cancel(&self) -> CameraHealth {
        invalidate(
            &self.control,
            "Cancellation requested; camera cleanup not yet confirmed",
        )
    }
    fn call(
        &self,
        request: Request,
        timeout: Duration,
        cancel: Option<&AtomicBool>,
    ) -> Result<Reply, CameraError> {
        if cancel.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            return Err(CameraError::Cancelled);
        }
        let health = self.health();
        if health.isolated && !health.cleanup_confirmed && !matches!(request, Request::Disconnect) {
            return Err(CameraError::Isolated(
                "cleanup remains unconfirmed; stale COM session cannot be reused".into(),
            ));
        }
        let context = CallContext {
            deadline: Instant::now() + timeout,
            epoch: self.control.generation.load(Ordering::Acquire),
            generation: self.control.generation.clone(),
        };
        let deadline = context.deadline;
        let epoch = context.epoch;
        let (reply, response) = mpsc::sync_channel(1);
        self.queue
            .try_send(Message {
                request,
                context,
                response: reply,
            })
            .map_err(|e| match e {
                mpsc::TrySendError::Full(_) => {
                    CameraError::Busy("bounded camera queue is full".into())
                }
                mpsc::TrySendError::Disconnected(_) => {
                    CameraError::Unavailable("camera owner stopped".into())
                }
            })?;
        loop {
            if cancel.is_some_and(|flag| flag.load(Ordering::Acquire)) {
                invalidate_epoch(&self.control, epoch, "Cancellation requested; camera cleanup not yet confirmed");
                return Err(CameraError::Cancelled);
            }
            if self.control.generation.load(Ordering::Acquire) != epoch {
                return Err(CameraError::Cancelled);
            }
            if Instant::now() >= deadline {
                invalidate_epoch(&self.control, epoch, "Camera owner deadline; camera cleanup not yet confirmed");
                return Err(CameraError::Timeout(
                    "camera owner deadline; COM session isolated; cleanup unconfirmed".into(),
                ));
            }
            match response.recv_timeout(Duration::from_millis(10)) {
                Ok(reply) => {
                    if self.control.generation.load(Ordering::Acquire) != epoch {
                        return Err(CameraError::Cancelled);
                    }
                    return reply;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => (),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    invalidate_epoch(&self.control, epoch, "Camera worker failed; camera cleanup not yet confirmed");
                    return Err(CameraError::Unavailable(
                        "camera worker failed; cleanup unconfirmed".into(),
                    ));
                }
            }
        }
    }
    pub fn discover_list(&self) -> Result<Vec<CameraIdentity>, CameraError> {
        match self.call(Request::Discover, COMMAND_TIMEOUT, None)? {
            Reply::Devices(d) => Ok(d),
            _ => unreachable!(),
        }
    }
    pub fn connect_selected(&self, device_id: &str) -> Result<CameraHealth, CameraError> {
        match self.call(Request::Connect(device_id.into()), COMMAND_TIMEOUT, None)? {
            Reply::Health(h) => Ok(h),
            _ => unreachable!(),
        }
    }
    pub fn connect(&self) -> Result<CameraHealth, CameraError> {
        Err(CameraError::Safety("explicit discover_list and connect_selected(device_id) required; implicit list selection forbidden".into()))
    }
    pub fn disconnect(&self) {
        let _ = self.disconnect_confirmed();
    }
    pub fn disconnect_confirmed(&self) -> Result<CameraHealth, CameraError> {
        self.request_cancel();
        match self.call(Request::Disconnect, COMMAND_TIMEOUT, None)? {
            Reply::Health(h) => Ok(h),
            _ => unreachable!(),
        }
    }
    pub fn read_properties(&self) -> Result<BTreeMap<String, CameraProperty>, CameraError> {
        match self.call(Request::Read, COMMAND_TIMEOUT, None)? {
            Reply::Properties(p) => Ok(p),
            _ => unreachable!(),
        }
    }
    pub fn validate_properties(&self, values: &CameraProperties) -> Result<(), CameraError> {
        validate_properties(&self.health(), values)
    }
    /// Classified admission decision for the domain entry: the caller reports the
    /// stable rejection code instead of one opaque error.
    pub fn property_rejection(
        &self,
        values: &CameraProperties,
    ) -> Result<(), crate::devices::camera::PropertyRejection> {
        crate::devices::camera::property_rejection(&self.health(), values)
    }
    pub fn apply_properties(
        &self,
        values: &CameraProperties,
    ) -> Result<PropertyApplyResult, CameraError> {
        self.validate_properties(values)?;
        if let Some(target) = values.get("shutter") {
            *self.control.shutter_target.lock().unwrap() = Some(target.clone());
        }
        match self.call(Request::Apply(values.clone()), COMMAND_TIMEOUT, None)? {
            Reply::Applied(a) => Ok(a),
            _ => unreachable!(),
        }
    }
    pub fn profile_readback(&self) -> Result<CameraProperties, CameraError> {
        profile_readback(&self.health())
    }
    pub fn set_exposure_ms(&self, milliseconds: f64) -> Result<String, CameraError> {
        let value = exact_shutter_value(&self.health(), milliseconds)?;
        let applied =
            self.apply_properties(&BTreeMap::from([("shutter".into(), value.clone())]))?;
        if !applied.complete {
            return Err(CameraError::Safety(
                "exact shutter write/readback did not complete".into(),
            ));
        }
        Ok(format!("native {:?}", value))
    }
    /// Shared target only. The explicit Apply/preflight path owns all I/O.
    pub fn set_exposure_target(&self, milliseconds: f64) -> Result<(), CameraError> {
        let target = exact_shutter_value(&self.health(), milliseconds)?;
        *self.control.shutter_target.lock().unwrap() = Some(target.clone());
        if let Some(property) = self
            .control
            .cached
            .lock()
            .unwrap()
            .properties
            .get_mut("shutter")
        {
            property.target = Some(target);
        }
        Ok(())
    }
    pub fn configure_host_capture(&self, milliseconds: f64) -> Result<String, CameraError> {
        let result = self.set_exposure_ms(milliseconds)?;
        self.call(Request::PrepareHost, COMMAND_TIMEOUT, None)?;
        self.profile_readback()?;
        Ok(result)
    }
    pub fn capture_to_path(
        &self,
        path: &Path,
        operation_id: &str,
        cancel: &AtomicBool,
        events: Option<mpsc::Sender<CameraCaptureEvent>>,
    ) -> Result<CapturedImage, CameraError> {
        self.capture_with_policy(path,operation_id,cancel,events,true)
    }
    pub fn capture_independent_to_path(&self,path:&Path,operation_id:&str,cancel:&AtomicBool)->Result<CapturedImage,CameraError>{
        self.capture_with_policy(path,operation_id,cancel,None,false)
    }
    fn capture_with_policy(&self,path:&Path,operation_id:&str,cancel:&AtomicBool,events:Option<mpsc::Sender<CameraCaptureEvent>>,scan_policy:bool)->Result<CapturedImage,CameraError>{
        if self.health().live_view.active {
            return Err(invalid("capture prohibited while Live View is active"));
        }
        match self.call(
            Request::Capture {
                path: path.into(),
                operation_id: operation_id.into(),
                events,
                scan_policy,
            },
            CAPTURE_TIMEOUT,
            Some(cancel),
        )? {
            Reply::Captured(image) => Ok(image),
            _ => unreachable!(),
        }
    }
    pub fn capture_frame_cancellable(
        &self,
        save_path: &Path,
        task_id: &str,
        index: u32,
        cancel: &AtomicBool,
    ) -> Result<PathBuf, CameraError> {
        validate_component(task_id)?;
        let path = save_path
            .join(task_id)
            .join("frames")
            .join(format!("frame-{index:04}.nef"));
        self.capture_to_path(
            &path,
            &format!("frame-{index}-{}", random_session()?),
            cancel,
            None,
        )
        .map(|i| i.path)
    }
    pub fn capture_frame(
        &self,
        save_path: &Path,
        task_id: &str,
        index: u32,
    ) -> Result<PathBuf, CameraError> {
        self.capture_frame_cancellable(save_path, task_id, index, &AtomicBool::new(false))
    }
    pub fn start_live_view(&self) -> Result<CameraHealth, CameraError> {
        match self.call(Request::LvStart(self.control.live_generation.load(Ordering::Acquire)), COMMAND_TIMEOUT, None)? {
            Reply::Health(h) => Ok(h),
            _ => unreachable!(),
        }
    }
    pub fn stop_live_view(&self) -> Result<CameraHealth, CameraError> {
        self.stop_live_view_with_timeout(COMMAND_TIMEOUT)
    }
    /// Bounded Live View stop used by the Stop path, which must end Live View
    /// **before** it cancels the session (see `enqueue_stop`): the ordinary Live
    /// View stop confirms OFF in well under a second, but it must never delay a
    /// safety Stop for a long budget if the camera worker is wedged.
    pub fn stop_live_view_with_timeout(&self, timeout: Duration) -> Result<CameraHealth, CameraError> {
        self.control.live_generation.fetch_add(1,Ordering::AcqRel);
        self.control.live_stop_pending.store(true,Ordering::Release);
        self.control.frames.revoke();
        clear_live_view_controls(&mut self.control.cached.lock().unwrap(),"LV停止已受理，旧控制会话已撤销");
        match self.call(Request::LvStop, timeout, None)? {
            Reply::Health(h) => Ok(h),
            _ => unreachable!(),
        }
    }
    pub fn read_live_view_controls(&self,session:&str)->Result<CameraHealth,CameraError>{
        match self.call(Request::LvRead(session.into(),self.control.live_generation.load(Ordering::Acquire)),LV_CONTROL_TIMEOUT,None)?{Reply::Health(h)=>Ok(h),_=>unreachable!()}
    }
    pub fn configure_live_view(&self,session:&str,controls:&CameraProperties)->Result<PropertyApplyResult,CameraError>{
        match self.call(Request::LvConfigure(session.into(),controls.clone(),self.control.live_generation.load(Ordering::Acquire)),LV_CONTROL_TIMEOUT,None)?{Reply::Applied(a)=>Ok(a),_=>unreachable!()}
    }
    pub fn autofocus(&self) -> Result<(), CameraError> {
        let health=self.health();let session=health.live_view.active.then(||health.live_view.session_id.clone()).flatten();
        match self.call(Request::Af(session,self.control.live_generation.load(Ordering::Acquire)), COMMAND_TIMEOUT, None)? {
            Reply::Done => Ok(()),
            _ => unreachable!(),
        }
    }
    pub fn focus_step(&self, step: i32) -> Result<(), CameraError> {
        let step = validate_focus_step(i64::from(step))?;
        let health=self.health();let session=health.live_view.session_id.clone().filter(|_|health.live_view.active).ok_or_else(||invalid("focus step requires current Live View session"))?;
        match self.call(Request::Focus(step,session,self.control.live_generation.load(Ordering::Acquire)), COMMAND_TIMEOUT, None)? {
            Reply::Done => Ok(()),
            _ => unreachable!(),
        }
    }
}

fn validate_component(value: &str) -> Result<(), CameraError> {
    if value.trim().is_empty() || matches!(value, "." | "..") || value.contains(['/', '\\', '\0']) {
        Err(invalid("task ID is not a safe path component"))
    } else {
        Ok(())
    }
}
fn invalidate(control: &Control, reason: &str) -> CameraHealth {
    invalidate_generation(control, None, reason)
}
fn invalidate_epoch(control: &Control, epoch: u64, reason: &str) -> CameraHealth {
    invalidate_generation(control, Some(epoch), reason)
}
fn invalidate_generation(control: &Control, expected_epoch: Option<u64>, reason: &str) -> CameraHealth {
    let _gate = control.commit_gate.lock().unwrap();
    // An old waiter may observe its cancellation after Stop already retired
    // the session. It must not revoke that newer generation's cleanup proof.
    if expected_epoch.is_some_and(|epoch| control.generation.load(Ordering::Acquire) != epoch) {
        return control.cached.lock().unwrap().clone();
    }
    control.generation.fetch_add(1, Ordering::AcqRel);
    control.live_generation.fetch_add(1,Ordering::AcqRel);
    control.live_stop_pending.store(true,Ordering::Release);
    control.frames.revoke();
    let mut h = control.cached.lock().unwrap();
    h.connected = false;
    h.isolated = true;
    h.cleanup_confirmed = false;
    h.phase = "cancel_requested".into();
    h.live_view.active = false;
    h.live_view.fresh = false;
    h.live_view.error = Some(reason.into());
    h.live_view.session_id=None;
    clear_live_view_controls(&mut h,reason);
    h.last_error = Some(reason.into());
    h.clone()
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use crate::devices::camera::{scripted_camera, ScriptedCameraConfig};

    #[test]
    fn retired_waiter_cannot_revoke_confirmed_stop_cleanup() {
        let camera = scripted_camera(ScriptedCameraConfig::default());
        let old_epoch = camera.control.generation.load(Ordering::Acquire);
        camera.disconnect_confirmed().unwrap();
        let retired_epoch = camera.control.generation.load(Ordering::Acquire);
        let health = invalidate_epoch(&camera.control, old_epoch, "late cancelled waiter");
        assert!(health.cleanup_confirmed && !health.connected && health.isolated);
        assert_eq!(camera.control.generation.load(Ordering::Acquire), retired_epoch);
        assert_ne!(health.last_error.as_deref(), Some("late cancelled waiter"));
    }

    #[test]
    fn current_waiter_failure_still_invalidates_and_stale_waiter_preserves_unknown() {
        let camera = scripted_camera(ScriptedCameraConfig { close_failure: true, ..Default::default() });
        let epoch = camera.control.generation.load(Ordering::Acquire);
        let health = invalidate_epoch(&camera.control, epoch, "current worker failed");
        assert!(!health.connected && health.isolated && !health.cleanup_confirmed);
        assert!(camera.disconnect_confirmed().is_err());
        let failed = camera.health();
        let health = invalidate_epoch(&camera.control, epoch, "late cancelled waiter");
        assert!(!health.cleanup_confirmed && health.isolated);
        assert_eq!(health.last_error, failed.last_error);
        assert_eq!(serde_json::to_value(&health.last_cleanup_diagnostic).unwrap(), serde_json::to_value(&failed.last_cleanup_diagnostic).unwrap());
    }
}
fn publish(camera: &NativeCamera, control: &Control, epoch: u64) {
    let mut cached = control.cached.lock().unwrap();
    if control.generation.load(Ordering::Acquire) == epoch {
        let mut health = camera.health.clone();
        refresh_property_controls(&mut health);
        if control.live_stop_pending.load(Ordering::Acquire){clear_live_view_controls(&mut health,"LV停止/取消已受理");health.live_view.fresh=false;}
        *cached = health;
    }
}
fn cleanup(camera: &mut NativeCamera, control: &Control, epoch: u64) {
    let trigger_error = camera.health.last_error.clone();
    control.frames.revoke();
    let session = camera.health.session_id.clone();
    if !camera.transport_open {
        // A no-op Cancel/Close cannot confirm an earlier failed retirement or
        // find a C102 on an interface already closed. Preserve its evidence.
        publish_cleanup(camera, control, &session);
        return;
    }
    let started = Instant::now();
    let pending_at_start = camera.capture_pending;
    let wait_limit = camera
        .pending_capture
        .as_ref()
        .filter(|receipt| {
            pending_at_start
                && receipt.queue_drained
                && Some(&receipt.session_id) == session.as_ref()
        })
        .map(|receipt| {
            Duration::from_secs_f64(receipt.exposure_ms / 1000.0) + Duration::from_secs(2)
        })
        .unwrap_or(Duration::from_secs(2));
    let operation = camera
        .pending_capture
        .as_ref()
        .map(|receipt| receipt.operation_id.clone())
        .or_else(|| camera.health.operation_id.clone());
    let mut diagnostic = CameraCleanupDiagnostic {
        session_id: session.clone(),
        operation_id: operation,
        pending_at_start,
        wait_limit_ms: wait_limit.as_millis() as u64,
        ..Default::default()
    };
    camera.health.connected = false;
    camera.health.isolated = true;
    camera.health.cleanup_confirmed = false;
    camera.health.phase = "cleanup_pending".into();
    camera.health.last_cleanup_diagnostic = Some(diagnostic.clone());
    publish_cleanup(camera, control, &session);
    let context = CallContext {
        deadline: Instant::now() + wait_limit,
        epoch,
        // Repeated Stop cancels business work immediately, but it cannot erase
        // the independent same-interface wait for physical termination proof.
        generation: Arc::new(AtomicU64::new(epoch)),
    };
    // This is the only place that can ever prove Live View OFF: once cleanup
    // publishes an unconfirmed session the transport is retired and there is no
    // retry path, so a single unlucky live-view transaction would isolate the
    // camera for the whole process lifetime. Confirm LV OFF with a bounded
    // retry on the still-usable transport, and keep the transport Cancel()
    // abort between the two attempts so a stalled in-flight transfer cannot
    // erase the only chance to establish the physical termination evidence.
    // The safety Stop waits only ~6 s for camera cleanup, so the cleanup-side
    // Live View confirmation must finish inside that window: fail fast, keep the
    // fail-closed verdict (session isolated, LV OFF unknown) and let
    // `transport.close()` release the device. `wait_limit` here is the
    // capture-termination budget and is already bounded; the retry after the
    // cancel gets a short slice because a cancelled session cannot confirm.
    let lv_off_budget = wait_limit.min(Duration::from_secs(2));
    let live_view_was_active = camera.health.live_view.active;
    let mut lv_stopped = true;
    let mut live_view_failure: Option<String> = None;
    let cancelled;
    if live_view_was_active {
        match confirm_live_view_off(camera, epoch, lv_off_budget) {
            Ok(()) => cancelled = camera.transport.cancel().is_ok(),
            Err(first) => {
                cancelled = camera.transport.cancel().is_ok();
                if let Err(retry) = confirm_live_view_off(camera, epoch, Duration::from_millis(750)) {
                    lv_stopped = false;
                    live_view_failure = Some(format!("{first}; after cancel: {retry}"));
                }
            }
        }
    } else {
        cancelled = camera.transport.cancel().is_ok();
    }
    diagnostic.io_cancel_confirmed = cancelled;
    if camera.capture_pending && camera.d7100 {
        // Discard cancelled objects. Only the reliable termination event can
        // clear an exposure whose start was accepted or uncertain.
        let mut associated = camera.pending_capture.as_ref().is_some_and(|receipt| {
            receipt.queue_drained && Some(&receipt.session_id) == session.as_ref()
        });
        while associated {
            if let Err(error) = context.check() {
                diagnostic.error = Some(error.to_string());
                break;
            }
            if camera.health.session_id != session {
                diagnostic.error = Some("camera session changed during cleanup".into());
                break;
            }
            diagnostic.poll_count += 1;
            match camera.events(&context) {
                Ok(events) => {
                    diagnostic.last_event_diagnostic = camera.health.last_event_diagnostic.clone();
                    if !events.is_empty() {
                        CameraEventBatchDiagnostic::record(
                            &mut diagnostic.event_batches,
                            diagnostic.poll_count,
                            events.len(),
                            camera
                                .health
                                .last_event_diagnostic
                                .clone()
                                .unwrap_or_default(),
                        );
                    }
                    let completions = events
                        .iter()
                        .filter(|event| event.code == CAPTURE_COMPLETE_RAM)
                        .count();
                    let unrelated = events.iter().any(|event| {
                        !matches!(
                            event.code,
                            CAPTURE_COMPLETE_RAM | OBJECT_RAM | 0x4006 | 0xc104
                        )
                    });
                    if unrelated || completions > 1 {
                        associated = false;
                        diagnostic.error=Some("cleanup received unassociated/reset/device-info or duplicate completion; termination proof refused".into());
                    } else if completions == 1 {
                        camera.capture_pending = false;
                        camera.pending_capture = None;
                        diagnostic.exposure_ended_observed = true;
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(CameraError::Busy(_)) => thread::sleep(Duration::from_millis(10)),
                Err(error) => {
                    diagnostic.last_event_diagnostic = camera.health.last_event_diagnostic.clone();
                    diagnostic.error = Some(error.to_string());
                    break;
                }
            }
        }
        if camera.capture_pending && diagnostic.error.is_none() {
            let accepted = camera
                .pending_capture
                .as_ref()
                .is_some_and(|receipt| receipt.accepted);
            diagnostic.error = Some(format!(
                "associated C102 absent for {} trigger; exposure termination remains unknown",
                if accepted { "accepted" } else { "uncertain" }
            ));
        }
    }
    let closed = camera.transport.close().is_ok();
    camera.transport_open = false;
    diagnostic.close_confirmed = closed;
    diagnostic.elapsed_ms = started.elapsed().as_millis() as u64;
    if !cancelled || !closed || !lv_stopped {
        // Preserve any capture-termination error already recorded and append the
        // real Live View transport error, so the one diagnostic field never hides
        // why the termination proof was refused.
        let detail = match live_view_failure.as_deref() {
            Some(detail) => format!("Live View OFF unconfirmed: {detail}"),
            None => "camera I/O cancel, Live View OFF or Close unconfirmed".into(),
        };
        diagnostic.error = Some(match diagnostic.error.take() {
            Some(existing) => format!("{existing}; {detail}"),
            None => detail,
        });
    }
    camera.health.last_cleanup_diagnostic = Some(diagnostic);
    camera.health.connected = false;
    camera.health.busy = false;
    camera.health.live_view.active = false;
    camera.health.live_view.fresh = false;
    camera.health.live_view.session_id=None;
    clear_live_view_controls(&mut camera.health,"相机断开/隔离，LV控制已撤销");
    camera.health.isolated = true;
    camera.health.cleanup_confirmed = cancelled && closed && lv_stopped && !camera.capture_pending;
    camera.health.phase = if camera.health.cleanup_confirmed {
        "disconnected"
    } else {
        "cleanup_unconfirmed"
    }
    .into();
    camera.health.last_error = if camera.health.cleanup_confirmed {
        // A confirmed cleanup must not keep the cancellation placeholder: it would
        // report an unconfirmed outcome on a session whose termination is proven.
        // A real earlier device error is still preserved.
        trigger_error.filter(|error| !error.contains("cleanup evidence required"))
    } else {
        let base = match trigger_error {
            Some(error) => format!(
                "{error}; camera cancellation/close or exposure termination remains unconfirmed"
            ),
            None => "Camera cancellation/close or exposure termination remains unconfirmed".into(),
        };
        Some(match live_view_failure.as_deref() {
            Some(detail) => format!("{base}; Live View OFF unconfirmed: {detail}"),
            None => base,
        })
    };
    publish_cleanup(camera, control, &session);
}
/// Bounded retry for the Live View OFF proof during a Stop. The native transport
/// reports a per-transaction error immediately, so a one-shot attempt can lose the
/// proof permanently; this retries until the caller's limit and returns the last
/// real transport error for diagnostics.
///
/// Order: `EndLiveView 0x9202` first, then the `LV_STATUS` readback, then a
/// best-effort `DeviceReady`. This deliberately differs from the reference
/// implementation's `DeviceReady(); EndLiveView; DeviceReady();`: on hardware the
/// *leading* `DeviceReady` is what blocks while a Live View stream is active,
/// whereas issuing `0x9202` first — the order the ordinary
/// `camera_live_view_stop` path already uses — confirms OFF. The trailing drain
/// is best-effort because a device still finishing a transfer may not answer it,
/// and that must not turn a confirmed stop into an unconfirmed one. Every step is
/// named in the returned error so a failure identifies the failing transaction
/// instead of only reporting a shared-deadline timeout.
fn confirm_live_view_off(
    camera: &mut NativeCamera,
    epoch: u64,
    limit: Duration,
) -> Result<(), CameraError> {
    let deadline = Instant::now() + limit;
    loop {
        let context = CallContext {
            deadline,
            epoch,
            generation: Arc::new(AtomicU64::new(epoch)),
        };
        let outcome = (|| -> Result<(), CameraError> {
            // Hardware evidence: with an active Live View stream the *leading*
            // DeviceReady is what blocks (the step-named diagnostic reports
            // "DeviceReady before end-Live-View: camera timeout"), while the
            // ordinary camera_live_view_stop path — which issues STOP_LV first and
            // never drains — confirms LV OFF in ~0.6 s on the same device. So the
            // end-Live-View command goes first and the drain becomes best-effort:
            // a device that is still finishing the transfer may not answer
            // DeviceReady, and that must not turn a confirmed stop into an
            // unconfirmed one.
            camera
                .transport
                .execute(STOP_LV, &[], DataPhase::None, &context)
                .and_then(PtpResponse::success)
                .map_err(|error| step_error("EndLiveView 0x9202", error))?;
            let state = camera
                .transport
                .execute(
                    GET_PROPERTY,
                    &[LV_STATUS as u32],
                    DataPhase::Read { max: 16 },
                    &context,
                )
                .and_then(PtpResponse::data)
                .map_err(|error| step_error("LiveViewStatus readback", error))?;
            if state != [0] {
                return Err(step_error(
                    "LiveViewStatus still active after end-Live-View",
                    CameraError::Command(format!("LV_STATUS={state:?}")),
                ));
            }
            let _ = camera.device_ready(&context);
            Ok(())
        })();
        match outcome {
            Ok(()) => return Ok(()),
            Err(error) => {
                if Instant::now() >= deadline {
                    return Err(error);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}
fn publish_cleanup(camera: &NativeCamera, control: &Control, session: &Option<String>) {
    let mut cached = control.cached.lock().unwrap();
    // Safety evidence may outlive Stop generations, but never crosses into a
    // newly selected physical session and never publishes capture business work.
    if camera.health.session_id == *session && cached.session_id == *session {
        let mut health = camera.health.clone();
        refresh_property_controls(&mut health);
        *cached = health;
    }
}
fn worker(mut camera: NativeCamera, receiver: mpsc::Receiver<Message>, control: Arc<Control>) {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut observed_epoch = control.generation.load(Ordering::Acquire);
        let mut next_frame = Instant::now();
        let mut next_health = Instant::now() + Duration::from_millis(250);
        loop {
            let epoch = control.generation.load(Ordering::Acquire);
            if epoch != observed_epoch {
                cleanup(&mut camera, &control, epoch);
                observed_epoch = epoch;
            }
            match receiver.recv_timeout(Duration::from_millis(10)) {
                Ok(message) => {
                    if let Err(e) = message.context.check() {
                        let _ = message.response.send(Err(e));
                        continue;
                    }
                    let op = match &message.request {
                        Request::Capture { operation_id, .. } => Some(operation_id.clone()),
                        _ => None,
                    };
                    camera.health.busy = true;
                    camera.health.operation_id = op;
                    camera.health.phase = "working".into();
                    if !matches!(message.request, Request::Connect(_)) {
                        if let Some(target) = control.shutter_target.lock().unwrap().clone() {
                            if let Some(property) = camera.health.properties.get_mut("shutter") {
                                property.target = Some(target);
                            }
                        }
                        // The worker's own health is what capture/scan-policy checks
                        // read, so the operator attestation must be merged here too.
                        // Validity against the live session is still decided fresh
                        // by the domain layer, never by this copy.
                        camera.health.operator_multiple_exposure =
                            control.operator_multiple_exposure.lock().unwrap().clone();
                    }
                    publish(&camera, &control, epoch);
                    *control.deadline.lock().unwrap() = Some((epoch, message.context.deadline));
                    let mut result =
                        execute(&mut camera, &control, &message.request, &message.context);
                    *control.deadline.lock().unwrap() = None;
                    camera.health.busy = false;
                    if !camera.health.live_view.active {
                        control.frames.revoke();
                    }
                    if let Err(e) = &result {
                        camera.health.last_error = Some(e.to_string());
                        // Validation rejection does not poison a healthy session.
                        // Once native I/O may be unresolved, retire the entire session.
                        if matches!(
                            e,
                            CameraError::Timeout(_)
                                | CameraError::Unavailable(_)
                                | CameraError::Busy(_)
                                | CameraError::DeviceBusy(_)
                                | CameraError::Io(_)
                                | CameraError::Cancelled
                        ) || camera.capture_pending
                            || matches!(
                                message.request,
                                Request::Capture { .. }
                                    | Request::Connect(_)
                                    | Request::LvStart(_)
                                    | Request::LvStop
                            )
                        {
                            let new_epoch = control.generation.load(Ordering::Acquire);
                            cleanup(&mut camera, &control, new_epoch);
                            observed_epoch = new_epoch;
                        }
                    } else if !matches!(&result, Ok(Reply::Applied(a)) if !a.complete) {
                        camera.health.last_error = None;
                    }
                    camera.health.phase = if camera.health.connected {
                        if camera.health.live_view.active {
                            "live_view"
                        } else {
                            "idle"
                        }
                    } else if camera.health.cleanup_confirmed {
                        "disconnected"
                    } else {
                        "cleanup_unconfirmed"
                    }
                    .into();
                    refresh_property_controls(&mut camera.health);
                    camera.refresh_live_view_permissions();
                    publish(&camera, &control, epoch);
                    if matches!(result, Ok(Reply::Health(_))) {
                        result = Ok(Reply::Health(camera.health.clone()));
                    }
                    let _ = message.response.send(result);
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    cleanup(&mut camera, &control, epoch);
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => (),
            }
            if camera.health.connected
                && camera.health.live_view.active
                && !control.live_stop_pending.load(Ordering::Acquire)
                && Instant::now() >= next_frame
            {
                next_frame = Instant::now() + LV_FRAME_INTERVAL;
                let epoch = control.generation.load(Ordering::Acquire);
                let live_epoch = control.live_generation.load(Ordering::Acquire);
                let context = CallContext {
                    deadline: Instant::now() + Duration::from_secs(2),
                    epoch,
                    generation: control.generation.clone(),
                };
                *control.deadline.lock().unwrap() = Some((epoch, context.deadline));
                let result = camera
                    .transport
                    .execute(
                        GET_LV,
                        &[],
                        DataPhase::Read {
                            max: MAX_LIVE_VIEW_BYTES + 384,
                        },
                        &context,
                    )
                    .and_then(PtpResponse::data)
                    .and_then(|data| {
                        d7100_frame(
                            &data,
                            camera.health.live_view.session_id.as_deref().unwrap_or_default(),
                            camera.health.live_view.sequence + 1,
                        )
                    });
                *control.deadline.lock().unwrap() = None;
                match result {
                    Ok(frame) if context.check().is_ok()
                        && control.live_generation.load(Ordering::Acquire) == live_epoch
                        && !control.live_stop_pending.load(Ordering::Acquire) => {
                        camera.health.live_view.sequence = frame.sequence;
                        camera.health.live_view.width = Some(frame.width);
                        camera.health.live_view.height = Some(frame.height);
                        camera.health.live_view.last_frame_at =
                            Some(chrono::Utc::now().to_rfc3339());
                        camera.health.live_view.fresh = true;
                        camera.health.live_view.error = None;
                        control.frames.publish(frame);
                        publish(&camera, &control, epoch);
                    }
                    Ok(_) => (),
                    Err(e) => {
                        camera.health.live_view.error = Some(e.to_string());
                        camera.health.last_error = Some(e.to_string());
                        control.generation.fetch_add(1, Ordering::AcqRel);
                        observed_epoch = control.generation.load(Ordering::Acquire);
                        cleanup(&mut camera, &control, observed_epoch);
                    }
                }
            }
            if camera.health.connected
                && !camera.health.live_view.active
                && Instant::now() >= next_health
            {
                next_health = Instant::now() + Duration::from_millis(250);
                let epoch = control.generation.load(Ordering::Acquire);
                let context = CallContext {
                    deadline: Instant::now() + Duration::from_secs(2),
                    epoch,
                    generation: control.generation.clone(),
                };
                *control.deadline.lock().unwrap() = Some((epoch, context.deadline));
                let result = poll_idle_health(&mut camera, &context);
                *control.deadline.lock().unwrap() = None;
                match result {
                    Ok(()) if context.check().is_ok() => {
                        camera.health.last_seen_at = Some(chrono::Utc::now().to_rfc3339());
                        publish(&camera, &control, epoch);
                    }
                    Ok(_) => (),
                    Err(e) => {
                        camera.health.last_error = Some(e.to_string());
                        invalidate(&control, &e.to_string());
                        observed_epoch = control.generation.load(Ordering::Acquire);
                        cleanup(&mut camera, &control, observed_epoch);
                    }
                }
            }
        }
    }));
    if outcome.is_err() {
        // Publish UNKNOWN before any potentially blocked COM Drop/Close. The
        // transport remains owned by this same thread throughout unwinding.
        invalidate(
            &control,
            "Camera owner panicked; cleanup unconfirmed; session isolated",
        );
        *control.deadline.lock().unwrap_or_else(|p| p.into_inner()) = None;
        let mut health = control.cached.lock().unwrap_or_else(|p| p.into_inner());
        health.busy = false;
        health.phase = "cleanup_unconfirmed".into();
    }
}

fn poll_idle_health(camera: &mut NativeCamera, context: &CallContext) -> Result<(), CameraError> {
    if camera.d7100
        && camera.info.as_ref().is_some_and(|info| {
            info.operations.contains(&DEVICE_READY) && info.operations.contains(&GET_EVENT)
        })
    {
        camera.device_ready(context)?;
        let mut refresh_properties = false;
        let mut refresh_runtime = false;
        let mut refresh_prohibit = false;
        let mut refresh_catalog = std::collections::BTreeSet::new();
        let events = camera.events(context)?;
        let refresh_device_info = events.iter().any(|event| event.code == DEVICE_INFO_CHANGED);
        if refresh_device_info {
            camera.health.last_device_info_changed_diagnostic =
                camera.health.last_event_diagnostic.clone();
        }
        for event in events {
            match event.code {
                DEVICE_INFO_CHANGED => (), // The fixed Nikon parameter slot is opaque, not an object handle.
                0x4006 => {
                    if event.parameter == INFO_DISPLAY_ERROR_STATUS as u32 {
                        refresh_runtime = true;
                        continue;
                    }
                    if event.parameter == LV_PROHIBIT as u32 {refresh_prohibit=true;continue;}
                    if let Ok(code)=u16::try_from(event.parameter) {
                        if RUNTIME_DIAGNOSTICS.iter().any(|row|row.2==code) {refresh_catalog.insert(code);continue;}
                    }
                    for property in camera
                        .health
                        .properties
                        .values_mut()
                        .filter(|p| p.code as u32 == event.parameter)
                    {
                        refresh_properties = true;
                        property.confirmed = false;
                        property.actual = None;
                        property.error = Some(
                            "Camera property changed externally; explicit readback required".into(),
                        );
                    }
                }
                0xc104 => (),
                _ => return Err(CameraError::Safety(format!("unassociated capture or device event while idle: code=0x{:04X}, parameter=0x{:08X}; session retired", event.code, event.parameter))),
            }
        }
        if refresh_device_info {
            let result = camera.refresh_device_info(context);
            if let Err(error) = &result {
                if let Some(diagnostic) = camera.health.last_device_info_changed_diagnostic.as_mut()
                {
                    diagnostic.error = Some(error.to_string());
                }
            }
            result?;
        } else if refresh_properties {
            // Read all changed acquisition values before the runtime status at
            // the end of read_properties. Notifications in one batch have no
            // intermediate UNKNOWN state on which to apply capture preconditions.
            camera.read_properties(context)?;
        } else {
            // Idle status is diagnostic: Card media or a nonzero error value
            // does not imply an active capture and must remain visible as read.
            if refresh_runtime {
                camera.read_runtime_status(context)?;
            }
            if refresh_prohibit {
                camera.read_live_view_prohibit(context)?;
            }
            for code in refresh_catalog {
                camera.read_catalog_runtime(code, context)?;
            }
        }
        Ok(())
    } else {
        let info = parse_device_info(
            &camera
                .transport
                .execute(
                    GET_DEVICE_INFO,
                    &[],
                    DataPhase::Read { max: 128 * 1024 },
                    context,
                )?
                .data()?,
        )?;
        let identity = camera
            .health
            .identity
            .as_ref()
            .ok_or_else(|| invalid("idle camera identity missing"))?;
        if info.model != identity.model
            || info.manufacturer != identity.manufacturer
            || (!info.serial.is_empty() && identity.serial.as_deref() != Some(&info.serial))
        {
            return Err(invalid("camera identity changed during selected session"));
        }
        Ok(())
    }
}

fn execute(
    camera: &mut NativeCamera,
    control: &Control,
    request: &Request,
    context: &CallContext,
) -> Result<Reply, CameraError> {
    match request {
        Request::Discover => {
            // Enumeration must survive a device that is still finishing a previous
            // operation (reference implementation retries the busy state). A busy
            // discovery otherwise fails the whole acquisition in milliseconds.
            let mut attempts = 0u32;
            let list = loop {
                match camera.transport.discover(context) {
                    Err(CameraError::DeviceBusy(message)) => {
                        if Instant::now() >= context.deadline || attempts >= 600 {
                            return Err(CameraError::Timeout(format!("device stayed busy: {message}")));
                        }
                        attempts += 1;
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    other => break other?,
                }
            };
            camera.health.discovered = list.clone();
            Ok(Reply::Devices(list))
        }
        Request::Connect(id) => {
            *control.shutter_target.lock().unwrap() = None;
            let session = random_session()?;
            // A device that is still finishing a previous operation reports
            // ERROR_BUSY; the reference implementation retries that state instead
            // of failing, and the whole acquisition (open + identity/capability
            // probes) must be retryable, not just the open call, otherwise a busy
            // device makes the workstation fail to re-acquire the camera and
            // forces an operator restart.
            let mut attempts = 0u32;
            loop {
                match camera.connect(id, &session, context) {
                    Err(CameraError::DeviceBusy(message)) => {
                        if Instant::now() >= context.deadline || attempts >= 600 {
                            return Err(CameraError::Timeout(format!("device stayed busy: {message}")));
                        }
                        attempts += 1;
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    other => {
                        other?;
                        break;
                    }
                }
            }
            Ok(Reply::Health(camera.health.clone()))
        }
        Request::Disconnect => {
            cleanup(camera, control, control.generation.load(Ordering::Acquire));
            if !camera.health.cleanup_confirmed {
                return Err(CameraError::Isolated("camera cleanup unconfirmed; retired transport cannot confirm prior exposure/close failure".into()));
            }
            Ok(Reply::Health(camera.health.clone()))
        }
        Request::Read => {
            camera.read_properties(context)?;
            Ok(Reply::Properties(camera.health.properties.clone()))
        }
        Request::Apply(values) => {
            if camera.health.live_view.active {
                return Err(invalid("stop Live View before applying camera parameters"));
            }
            Ok(Reply::Applied(camera.apply(values, context)?))
        }
        Request::PrepareHost => {
            camera.prepare_host_capture(context)?;
            Ok(Reply::Done)
        }
        Request::Capture {
            path,
            operation_id,
            events,
            scan_policy,
        } => {
            let image = capture_with_policy(
                camera,
                path,
                operation_id,
                events,
                context,
                &control.commit_gate,
                *scan_policy,
            )?;
            camera.health.last_capture = Some(image.path.display().to_string());
            Ok(Reply::Captured(image))
        }
        Request::LvStart(live_epoch) => {
            camera.ensure_connected()?;
            if !camera.health.capabilities.live_view {
                return Err(CameraError::Unsupported(
                    "D7100 Live View is not supported".into(),
                ));
            }
            if camera.health.live_view.active {
                return Ok(Reply::Health(camera.health.clone()));
            }
            if control.live_generation.load(Ordering::Acquire)!=*live_epoch{return Err(CameraError::Unsupported("旧LV启动请求已被Stop撤销".into()));}
            control.live_stop_pending.store(false,Ordering::Release);
            let guard=||if control.live_generation.load(Ordering::Acquire)==*live_epoch&&!control.live_stop_pending.load(Ordering::Acquire){Ok(())}else{Err(CameraError::Unsupported("旧LV控制会话已被Stop撤销".into()))};
            camera.device_ready(context)?;
            guard()?;
            camera
                .transport
                .execute(START_LV, &[], DataPhase::None, context)?
                .success()?;
            let state = camera
                .transport
                .execute(
                    GET_PROPERTY,
                    &[LV_STATUS as u32],
                    DataPhase::Read { max: 16 },
                    context,
                )?
                .data()?;
            if state != [1] {
                return Err(invalid("Live View start was not confirmed"));
            }
            guard()?;
            let session=random_session()?;
            start_frame_pipe(control.frames.clone(), &session)?;
            let sequence = camera.health.live_view.sequence;
            camera.health.live_view = LiveViewStatus {
                active: true,
                session_id: Some(session.clone()),
                sequence,
                ..LiveViewStatus::default()
            };
            camera.health.live_view_controls=CameraLiveViewControls::default();
            camera.read_live_view_controls(&session,context,&guard)?;
            Ok(Reply::Health(camera.health.clone()))
        }
        Request::LvStop => {
            camera.ensure_connected()?;
            control.frames.revoke();
            camera
                .transport
                .execute(STOP_LV, &[], DataPhase::None, context)?
                .success()?;
            let state = camera
                .transport
                .execute(
                    GET_PROPERTY,
                    &[LV_STATUS as u32],
                    DataPhase::Read { max: 16 },
                    context,
                )?
                .data()?;
            if state != [0] {
                return Err(invalid("Live View stop was not confirmed"));
            }
            camera.health.live_view.active = false;
            camera.health.live_view.fresh = false;
            camera.health.live_view.error = None;
            camera.health.live_view.session_id=None;
            clear_live_view_controls(&mut camera.health,"Live View已确认停止");
            Ok(Reply::Health(camera.health.clone()))
        }
        Request::LvRead(session,live_epoch)=>{
            let guard=||if control.live_generation.load(Ordering::Acquire)==*live_epoch&&!control.live_stop_pending.load(Ordering::Acquire){Ok(())}else{Err(CameraError::Unsupported("旧LV读取已被Stop撤销".into()))};
            camera.read_live_view_controls(session,context,&guard)?;Ok(Reply::Health(camera.health.clone()))
        },
        Request::LvConfigure(session,controls,live_epoch)=>{
            let guard=||if control.live_generation.load(Ordering::Acquire)==*live_epoch&&!control.live_stop_pending.load(Ordering::Acquire){Ok(())}else{Err(CameraError::Unsupported("旧LV配置已被Stop撤销；未继续写入".into()))};
            Ok(Reply::Applied(camera.configure_live_view(session,controls,context,&guard)?))
        },
        Request::Af(expected_session,live_epoch) => {
            camera.ensure_connected()?;
            let guard=||if control.live_generation.load(Ordering::Acquire)==*live_epoch&&(expected_session.is_none()||!control.live_stop_pending.load(Ordering::Acquire)){Ok(())}else{Err(CameraError::Unsupported("旧对焦请求已被LV Stop撤销".into()))};
            guard()?;
            if camera.health.live_view.session_id.as_ref()!=expected_session.as_ref(){return Err(CameraError::Unsupported("AF请求跨越Live View会话".into()));}
            camera.read_properties(context)?;
            if camera.health.live_view.active{let session=camera.health.live_view.session_id.clone().ok_or_else(||invalid("LV会话未知"))?;camera.read_live_view_controls(&session,context,&guard)?;}
            if !camera.health.capabilities.autofocus {
                return Err(CameraError::Unsupported(
                    "AF operation not advertised".into(),
                ));
            }
            camera.device_ready(context)?;
            guard()?;
            camera
                .transport
                .execute(AF_DRIVE, &[], DataPhase::None, context)?
                .success()?;
            camera.device_ready(context)?;
            Ok(Reply::Done)
        }
        Request::Focus(step,session,live_epoch) => {
            camera.ensure_connected()?;
            camera.read_properties(context)?;
            let guard=||if control.live_generation.load(Ordering::Acquire)==*live_epoch&&!control.live_stop_pending.load(Ordering::Acquire){Ok(())}else{Err(CameraError::Unsupported("旧LV步进已被Stop撤销".into()))};camera.read_live_view_controls(session,context,&guard)?;
            if !camera.health.capabilities.focus_step || !camera.health.live_view.active {
                return Err(CameraError::Unsupported(
                    "lens stepping requires advertised Nikon operation and active Live View".into(),
                ));
            }
            camera.device_ready(context)?;
            guard()?;
            camera
                .transport
                .execute(
                    FOCUS_DRIVE,
                    &[if *step > 0 { 2 } else { 1 }, step.unsigned_abs()],
                    DataPhase::None,
                    context,
                )?
                .success()?;
            camera.device_ready(context)?;
            Ok(Reply::Done)
        }
    }
}
