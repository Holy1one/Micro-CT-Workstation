//! Single serial owner with cached readback, independent monitoring and priority OFF.
use super::moxtek::RawMoxtekAdapter;
use super::{XrayError, XrayHealth};
use std::sync::{mpsc, Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

type OffReply = mpsc::Sender<Result<XrayHealth, XrayError>>;
type Job = Box<dyn FnOnce(&mut WorkerState, Arc<AtomicBool>) + Send>;
struct WorkerState { raw: RawMoxtekAdapter, deadline: Option<Instant>, fault: Option<String> }
struct OwnerLiveness {health:Arc<Mutex<XrayHealth>>,retired:bool}
impl Drop for OwnerLiveness {
    fn drop(&mut self) {
        if !self.retired {
            let mut health=self.health.lock().unwrap_or_else(|poison|poison.into_inner());
            health.connected=false;health.beam_off_confirmed=false;
            health.last_error=Some("Moxtek owner exited unexpectedly; output UNKNOWN and OFF cleanup unconfirmed".into());
        }
    }
}
struct Shared {
    jobs: mpsc::SyncSender<(u64, Job)>,
    off: mpsc::Sender<OffReply>,
    health: Arc<Mutex<XrayHealth>>,
    epoch: Arc<AtomicU64>,
    active_cancel: Arc<Mutex<Option<Arc<AtomicBool>>>>,
}

/// Clones are message proxies; the raw adapter and serial transport never leave the worker.
#[derive(Clone)]
pub struct MoxtekAdapter { shared: Arc<Shared> }
impl Default for MoxtekAdapter { fn default() -> Self { Self::new() } }
impl MoxtekAdapter {
    pub fn new() -> Self { Self::from_raw(RawMoxtekAdapter::new()) }
    pub(super) fn from_raw(mut raw: RawMoxtekAdapter) -> Self {
        let (jobs, receiver) = mpsc::sync_channel::<(u64, Job)>(32);
        let (off, off_receiver) = mpsc::channel::<OffReply>();
        let health = Arc::new(Mutex::new(raw.health()));
        raw.observe_health(health.clone());
        let epoch = Arc::new(AtomicU64::new(0));
        let active_cancel = Arc::new(Mutex::new(None::<Arc<AtomicBool>>));
        let shared = Arc::new(Shared { jobs, off, health: health.clone(), epoch: epoch.clone(), active_cancel: active_cancel.clone() });
        std::thread::Builder::new().name("moxtek-owner".into()).spawn(move || {
            let mut state = WorkerState { raw, deadline: None, fault: None };
            let mut liveness=OwnerLiveness{health:health.clone(),retired:false};
            let mut last_poll = Instant::now();
            loop {
                while let Ok(reply) = off_receiver.try_recv() {
                    state.raw.set_operation_cancel(None);
                    let result = state.raw.force_off();
                    state.deadline = None;
                    let mut value=state.raw.health();if state.fault.is_some(){value.last_error=state.fault.clone();}
                    *health.lock().unwrap_or_else(|p| p.into_inner()) = value;
                    let _ = reply.send(result);
                }
                if state.raw.health().connected {
                    let emitting = state.raw.health().beam_on;
                    let interval = if emitting { Duration::from_millis(250) } else { Duration::from_secs(2) };
                    if state.deadline.is_some_and(|limit| Instant::now() >= limit) {
                        epoch.fetch_add(1, Ordering::SeqCst);
                        let result = state.raw.force_off();
                        state.deadline = None;
                        let mut value = state.raw.health();
                        state.fault = Some(match result { Ok(_) => "Continuous X-ray hard limit reached; OFF confirmed".into(), Err(error) => format!("Continuous X-ray hard limit reached; OFF unconfirmed: {error}") });
                        value.last_error = state.fault.clone();
                        *health.lock().unwrap_or_else(|p| p.into_inner()) = value;
                    } else if last_poll.elapsed() >= interval {
                        let result = if emitting { state.raw.refresh_emission_status() } else { state.raw.refresh_status() };
                        if result.is_err() || (!emitting && state.raw.health().beam_on) {
                            state.fault=Some(result.as_ref().err().map(|e|e.to_string()).unwrap_or_else(||"Unexpected beam ON outside a verified operation".into()));
                            epoch.fetch_add(1, Ordering::SeqCst);
                            let _ = state.raw.force_off();
                        }
                        let mut value = state.raw.health();
                        if state.fault.is_some() { value.last_error = state.fault.clone(); }
                        *health.lock().unwrap_or_else(|p| p.into_inner()) = value;
                        last_poll = Instant::now();
                    }
                }
                match receiver.recv_timeout(Duration::from_millis(10)) {
                    Ok((generation, job)) => {
                        let cancel = Arc::new(AtomicBool::new(generation != epoch.load(Ordering::SeqCst)));
                        *active_cancel.lock().unwrap_or_else(|p| p.into_inner()) = Some(cancel.clone());
                        if generation != epoch.load(Ordering::SeqCst) { cancel.store(true, Ordering::SeqCst); }
                        state.raw.set_operation_cancel(Some(cancel.clone()));
                        job(&mut state, cancel);
                        state.raw.set_operation_cancel(None);
                        state.raw.set_external_cancel(None);
                        *active_cancel.lock().unwrap_or_else(|p| p.into_inner()) = None;
                        let mut value=state.raw.health();if state.fault.is_some(){value.last_error=state.fault.clone();}
                        *health.lock().unwrap_or_else(|p| p.into_inner()) = value;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {},
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            let _ = state.raw.disconnect();
            liveness.retired=true;
        }).expect("Moxtek worker must start");
        Self { shared }
    }
    pub fn health(&self) -> XrayHealth { self.shared.health.lock().unwrap_or_else(|p| p.into_inner()).clone() }
    fn call<T: Send + 'static>(&self, job: impl FnOnce(&mut WorkerState, Arc<AtomicBool>) -> Result<T, XrayError> + Send + 'static, external_cancel: Option<&AtomicBool>) -> Result<T, XrayError> {
        let (reply, receive) = mpsc::channel();
        let health=self.shared.health.clone();
        let generation = self.shared.epoch.load(Ordering::SeqCst);
        self.shared.jobs.try_send((generation, Box::new(move |state, token| {
            let result = if token.load(Ordering::SeqCst) { Err(XrayError::Cancelled) } else { job(state, token) };
            if let Err(error)=result.as_ref(){if matches!(error,XrayError::Transport(_)|XrayError::Protocol(_)|XrayError::Safety(_)){state.fault=Some(error.to_string());}}
            let mut value=state.raw.health();if state.fault.is_some(){value.last_error=state.fault.clone();}
            *health.lock().unwrap_or_else(|p|p.into_inner())=value;
            let _ = reply.send(result);
        }))).map_err(|_| XrayError::Transport("Moxtek queue full or worker unavailable".into()))?;
        let limit = Instant::now() + Duration::from_secs(30);
        loop {
            if external_cancel.is_some_and(|cancel| cancel.load(Ordering::SeqCst)) {
                let _ = self.request_off();
                return Err(XrayError::Cancelled);
            }
            match receive.recv_timeout(Duration::from_millis(20)) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(XrayError::Transport("Moxtek worker exited".into())),
                Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() >= limit => {
                    let _ = self.request_off();
                    return Err(XrayError::Transport("Moxtek operation deadline expired; OFF requested".into()));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {},
            }
        }
    }
    /// Nonblocking safety admission. Invalidates queued ON before waiting for serial I/O.
    pub fn request_off(&self) -> mpsc::Receiver<Result<XrayHealth, XrayError>> {
        self.shared.epoch.fetch_add(1, Ordering::SeqCst);
        if let Some(cancel) = self.shared.active_cancel.lock().unwrap_or_else(|p| p.into_inner()).as_ref() { cancel.store(true, Ordering::SeqCst); }
        let (reply, receive) = mpsc::channel();
        let _ = self.shared.off.send(reply);
        receive
    }
    pub fn force_off(&mut self) -> Result<XrayHealth, XrayError> { self.request_off().recv_timeout(Duration::from_secs(6)).map_err(|_| XrayError::Transport("Priority OFF cleanup unconfirmed".into()))? }
    pub fn discover_and_connect(&mut self) -> Result<XrayHealth, XrayError> { self.call(|s,_| s.raw.discover_and_connect(), None) }
    pub fn disconnect(&mut self) -> Result<(), XrayError> { self.force_off()?; self.call(|s,_| s.raw.disconnect(), None) }
    pub fn set_parameters(&mut self, kv: f64, ua: f64) -> Result<XrayHealth, XrayError> { self.call(move |s,_| {let health=s.raw.set_parameters(kv,ua)?;s.fault=None;Ok(health)}, None) }
    pub fn beam_on(&mut self, cancel: &AtomicBool) -> Result<XrayHealth, XrayError> { self.beam_on_until(cancel, Instant::now()+Duration::from_secs(600)) }
    pub fn beam_on_until(&mut self, cancel: &AtomicBool, deadline: Instant) -> Result<XrayHealth, XrayError> { self.call(move |s,token| { if let Some(fault)=s.fault.as_ref(){return Err(XrayError::Safety(fault.clone()));}s.deadline=Some(deadline); s.raw.beam_on_until(&token,deadline) }, Some(cancel)) }
    pub fn beam_on_shared(&mut self,cancel:Arc<AtomicBool>,deadline:Instant)->Result<XrayHealth,XrayError> {
        self.call(move |s,token|{if let Some(fault)=s.fault.as_ref(){return Err(XrayError::Safety(fault.clone()));}s.raw.set_external_cancel(Some(cancel.clone()));s.deadline=Some(deadline);s.raw.beam_on_until(&token,deadline)},None)
    }
    pub fn refresh_status(&mut self) -> Result<XrayHealth, XrayError> { self.call(|s,_| s.raw.refresh_status(), None) }
    pub fn refresh_off_status(&mut self) -> Result<XrayHealth, XrayError> { self.call(|s,_| s.raw.refresh_off_status(), None) }
    pub fn refresh_emission_status(&mut self) -> Result<XrayHealth, XrayError> { self.call(|s,_| s.raw.refresh_emission_status(), None) }
    pub fn refresh_emission_status_shared(&mut self,cancel:Arc<AtomicBool>)->Result<XrayHealth,XrayError>{self.call(move|s,_|{s.raw.set_external_cancel(Some(cancel));s.raw.refresh_emission_status()},None)}
    pub fn set_usb_auto_shutdown(&mut self, enabled: bool) -> Result<XrayHealth, XrayError> { self.call(move |s,_| s.raw.set_usb_auto_shutdown(enabled), None) }
    pub fn read_usb_shutdown_timer(&mut self) -> Result<u32, XrayError> { self.call(|s,_| s.raw.read_usb_shutdown_timer(), None) }
    pub fn set_usb_shutdown_delay(&mut self, delay:u16) -> Result<XrayHealth, XrayError> { self.call(move |s,_| s.raw.set_usb_shutdown_delay(delay), None) }
}
