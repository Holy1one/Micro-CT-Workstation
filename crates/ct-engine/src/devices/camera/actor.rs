//! Single camera/COM owner. Callers hold only cloneable Send message proxies.
//! Stop invalidates a generation immediately; blocking COM cannot delay OFF or
//! state reads and its late return cannot publish, capture or reopen Live View.

use super::capture::capture;
use super::live_view::*;
use super::nikon::*;
use super::ptp::*;
use super::types::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(75);
const LV_FRAME_INTERVAL: Duration = Duration::from_millis(100);

enum Request {
    Discover,
    Connect(String),
    Disconnect,
    Read,
    Apply(CameraProperties),
    Capture {
        path: PathBuf,
        operation_id: String,
        events: Option<mpsc::Sender<CameraCaptureEvent>>,
    },
    LvStart,
    LvStop,
    Af,
    Focus(i32),
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
    frames: LatestFrameSlot,
    commit_gate: Mutex<()>,
    deadline: Mutex<Option<(u64, Instant)>>,
    shutter_target: Mutex<Option<CameraValue>>,
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
            frames: LatestFrameSlot::default(),
            commit_gate: Mutex::new(()),
            deadline: Mutex::new(None),
            shutter_target: Mutex::new(None),
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
        h
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
                self.request_cancel();
                return Err(CameraError::Cancelled);
            }
            if self.control.generation.load(Ordering::Acquire) != epoch {
                return Err(CameraError::Cancelled);
            }
            if Instant::now() >= deadline {
                self.request_cancel();
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
                    self.request_cancel();
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
        if self.health().live_view.active {
            return Err(invalid("capture prohibited while Live View is active"));
        }
        match self.call(
            Request::Capture {
                path: path.into(),
                operation_id: operation_id.into(),
                events,
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
        match self.call(Request::LvStart, COMMAND_TIMEOUT, None)? {
            Reply::Health(h) => Ok(h),
            _ => unreachable!(),
        }
    }
    pub fn stop_live_view(&self) -> Result<CameraHealth, CameraError> {
        match self.call(Request::LvStop, COMMAND_TIMEOUT, None)? {
            Reply::Health(h) => Ok(h),
            _ => unreachable!(),
        }
    }
    pub fn autofocus(&self) -> Result<(), CameraError> {
        match self.call(Request::Af, COMMAND_TIMEOUT, None)? {
            Reply::Done => Ok(()),
            _ => unreachable!(),
        }
    }
    pub fn focus_step(&self, step: i32) -> Result<(), CameraError> {
        if step == 0 || !(-500..=500).contains(&step) {
            return Err(invalid(
                "focus step must be a nonzero signed native step within +/-500",
            ));
        }
        match self.call(Request::Focus(step), COMMAND_TIMEOUT, None)? {
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
    {
        let _gate = control.commit_gate.lock().unwrap();
        control.generation.fetch_add(1, Ordering::AcqRel);
    }
    control.frames.revoke();
    let mut h = control.cached.lock().unwrap();
    h.connected = false;
    h.isolated = true;
    h.cleanup_confirmed = false;
    h.phase = "cancel_requested".into();
    h.live_view.active = false;
    h.live_view.fresh = false;
    h.live_view.error = Some(reason.into());
    h.last_error = Some(reason.into());
    h.clone()
}
fn publish(camera: &NativeCamera, control: &Control, epoch: u64) {
    let mut cached = control.cached.lock().unwrap();
    if control.generation.load(Ordering::Acquire) == epoch {
        *cached = camera.health.clone();
    }
}
fn cleanup(camera: &mut NativeCamera, control: &Control, epoch: u64) {
    control.frames.revoke();
    let cancelled = camera.transport.cancel().is_ok();
    let context = CallContext {
        deadline: Instant::now() + Duration::from_secs(2),
        epoch,
        generation: control.generation.clone(),
    };
    let lv_stopped = if camera.health.live_view.active {
        camera
            .transport
            .execute(STOP_LV, &[], DataPhase::None, &context)
            .and_then(PtpResponse::success)
            .and_then(|_| {
                camera
                    .transport
                    .execute(
                        GET_PROPERTY,
                        &[LV_STATUS as u32],
                        DataPhase::Read { max: 16 },
                        &context,
                    )?
                    .data()
            })
            .is_ok_and(|state| state == [0])
    } else {
        true
    };
    if camera.capture_pending && camera.d7100 {
        // Discard cancelled objects. Only the reliable termination event can
        // clear an exposure whose start was accepted or uncertain.
        while context.check().is_ok() {
            match camera.events(&context) {
                Ok(events)
                    if events
                        .iter()
                        .any(|event| event.code == CAPTURE_COMPLETE_RAM) =>
                {
                    camera.capture_pending = false;
                    break;
                }
                Ok(_) => thread::sleep(Duration::from_millis(10)),
                Err(_) => break,
            }
        }
    }
    let closed = camera.transport.close().is_ok();
    camera.health.connected = false;
    camera.health.busy = false;
    camera.health.live_view.active = false;
    camera.health.live_view.fresh = false;
    camera.health.isolated = true;
    camera.health.cleanup_confirmed = cancelled && closed && lv_stopped && !camera.capture_pending;
    camera.health.phase = if camera.health.cleanup_confirmed {
        "disconnected"
    } else {
        "cleanup_unconfirmed"
    }
    .into();
    camera.health.last_error = (!camera.health.cleanup_confirmed)
        .then(|| "Camera cancellation/close or exposure termination remains unconfirmed".into());
    publish(camera, control, epoch);
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
                    }
                    publish(&camera, &control, epoch);
                    *control.deadline.lock().unwrap() = Some((epoch, message.context.deadline));
                    let mut result =
                        execute(&mut camera, &control, &message.request, &message.context);
                    *control.deadline.lock().unwrap() = None;
                    camera.health.busy = false;
                    if let Err(e) = &result {
                        camera.health.last_error = Some(e.to_string());
                        // Validation rejection does not poison a healthy session.
                        // Once native I/O may be unresolved, retire the entire session.
                        if matches!(
                            e,
                            CameraError::Timeout(_)
                                | CameraError::Unavailable(_)
                                | CameraError::Io(_)
                                | CameraError::Cancelled
                        ) || camera.capture_pending
                            || matches!(
                                message.request,
                                Request::Capture { .. }
                                    | Request::Connect(_)
                                    | Request::LvStart
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
                && Instant::now() >= next_frame
            {
                next_frame = Instant::now() + LV_FRAME_INTERVAL;
                let epoch = control.generation.load(Ordering::Acquire);
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
                            camera.health.session_id.as_deref().unwrap_or_default(),
                            camera.health.live_view.sequence + 1,
                        )
                    });
                *control.deadline.lock().unwrap() = None;
                match result {
                    Ok(frame) if context.check().is_ok() => {
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
    if camera.d7100 {
        camera
            .transport
            .execute(DEVICE_READY, &[], DataPhase::None, context)?
            .success()?;
        for event in camera.events(context)? {
            match event.code {
                0x4006 => {
                    for property in camera
                        .health
                        .properties
                        .values_mut()
                        .filter(|p| p.code as u32 == event.parameter)
                    {
                        property.confirmed = false;
                        property.actual = None;
                        property.error = Some(
                            "Camera property changed externally; explicit readback required".into(),
                        );
                    }
                }
                0xc104 => (),
                _ => {
                    return Err(invalid(
                        "unassociated capture or device event while idle; session retired",
                    ))
                }
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
            let list = camera.transport.discover(context)?;
            camera.health.discovered = list.clone();
            Ok(Reply::Devices(list))
        }
        Request::Connect(id) => {
            *control.shutter_target.lock().unwrap() = None;
            let session = random_session()?;
            camera.connect(id, &session, context)?;
            Ok(Reply::Health(camera.health.clone()))
        }
        Request::Disconnect => {
            camera.transport.close()?;
            camera.health.connected = false;
            camera.health.cleanup_confirmed = !camera.capture_pending;
            camera.health.isolated = !camera.health.cleanup_confirmed;
            control.frames.revoke();
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
        Request::Capture {
            path,
            operation_id,
            events,
        } => {
            let image = capture(
                camera,
                path,
                operation_id,
                events,
                context,
                &control.commit_gate,
            )?;
            camera.health.last_capture = Some(image.path.display().to_string());
            Ok(Reply::Captured(image))
        }
        Request::LvStart => {
            camera.ensure_connected()?;
            if !camera.health.capabilities.live_view {
                return Err(CameraError::Unsupported(
                    "D7100 Live View is not supported".into(),
                ));
            }
            if camera.health.live_view.active {
                return Ok(Reply::Health(camera.health.clone()));
            }
            camera.device_ready(context)?;
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
            let session = camera
                .health
                .session_id
                .as_deref()
                .ok_or_else(|| invalid("Live View session missing"))?;
            start_frame_pipe(control.frames.clone(), session)?;
            let sequence = camera.health.live_view.sequence;
            camera.health.live_view = LiveViewStatus {
                active: true,
                session_id: Some(session.into()),
                sequence,
                ..LiveViewStatus::default()
            };
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
            Ok(Reply::Health(camera.health.clone()))
        }
        Request::Af => {
            camera.ensure_connected()?;
            if !camera.health.capabilities.autofocus {
                return Err(CameraError::Unsupported(
                    "AF operation not advertised".into(),
                ));
            }
            camera.device_ready(context)?;
            camera
                .transport
                .execute(AF_DRIVE, &[], DataPhase::None, context)?
                .success()?;
            camera.device_ready(context)?;
            Ok(Reply::Done)
        }
        Request::Focus(step) => {
            camera.ensure_connected()?;
            if !camera.health.capabilities.focus_step || !camera.health.live_view.active {
                return Err(CameraError::Unsupported(
                    "lens stepping requires advertised Nikon operation and active Live View".into(),
                ));
            }
            camera.device_ready(context)?;
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
