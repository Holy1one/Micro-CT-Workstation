use serde::Serialize;
use serialport::{SerialPort, SerialPortInfo, SerialPortType};
use std::collections::HashMap;
#[cfg(test)]
use std::collections::VecDeque;
use std::fmt;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const BAUD_RATE: u32 = 115_200;
pub const EXPECTED_DEVICE: &str = "RTS9060_NANO";
pub const EXPECTED_PROTOCOL: u32 = 2;
const REQUIRED_CAPABILITIES: [&str; 2] = ["HEARTBEAT_ACK", "XRAY_WARNING"];
const HEARTBEAT_INTERVAL: Duration = Duration::from_millis(500);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_millis(1_500);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const HOME_TIMEOUT: Duration = Duration::from_secs(300);
const MOVE_TIMEOUT: Duration = Duration::from_secs(300);
const STOP_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_LINE_BYTES: usize = 512;
const CH340_IDS: [(u16, u16); 4] = [
    (0x1A86, 0x7523),
    (0x1A86, 0x5523),
    (0x1A86, 0x55D4),
    (0x4348, 0x5523),
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NanoIdentity {
    pub device: String,
    pub version: String,
    pub protocol: u32,
    pub build: String,
    pub buzzer_installed: bool,
    pub capabilities: Vec<String>,
}

impl NanoIdentity {
    fn require_supported(&self) -> Result<(), NanoError> {
        if self.device != EXPECTED_DEVICE {
            return Err(NanoError::Identity(format!(
                "unexpected device {}; expected {EXPECTED_DEVICE}",
                self.device
            )));
        }
        if self.protocol != EXPECTED_PROTOCOL {
            return Err(NanoError::Identity(format!(
                "unsupported protocol {}; expected {EXPECTED_PROTOCOL}",
                self.protocol
            )));
        }
        if !self.buzzer_installed {
            return Err(NanoError::Identity(
                "required warning buzzer is not installed".into(),
            ));
        }
        for capability in REQUIRED_CAPABILITIES {
            if !self.capabilities.iter().any(|value| value == capability) {
                return Err(NanoError::Identity(format!(
                    "missing required capability {capability}"
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NanoStatus {
    pub state: String,
    pub position_pulses: i64,
    pub target_pulses: i64,
    pub microsteps: u32,
    pub pulses_per_rev: u32,
    pub reference_valid: bool,
    pub homed: bool,
    pub rearmed: bool,
    pub hall_active: bool,
    pub capture_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveTicket {
    pub command_id: u32,
    pub angle_mdeg: i32,
    pub position_pulses: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NanoConnectionState {
    Disconnected,
    Discovering,
    Connected,
    Lost,
    Fault,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NanoHealth {
    pub state: NanoConnectionState,
    pub port: Option<String>,
    pub identity: Option<NanoIdentity>,
    pub status: Option<NanoStatus>,
    pub last_heartbeat_rtt_ms: Option<u64>,
    pub last_error: Option<String>,
}

impl Default for NanoHealth {
    fn default() -> Self {
        Self {
            state: NanoConnectionState::Disconnected,
            port: None,
            identity: None,
            status: None,
            last_heartbeat_rtt_ms: None,
            last_error: None,
        }
    }
}

#[derive(Debug)]
pub enum NanoError {
    NotConnected,
    Busy,
    NoDevice,
    Ambiguous(Vec<String>),
    Transport(String),
    Timeout(String),
    Protocol(String),
    Identity(String),
    Safety(String),
}

impl fmt::Display for NanoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConnected => write!(formatter, "Nano is not connected"),
            Self::Busy => write!(formatter, "Nano adapter already has an active session"),
            Self::NoDevice => write!(formatter, "no compatible RTS9060 Nano was found"),
            Self::Ambiguous(ports) => write!(
                formatter,
                "multiple compatible RTS9060 Nano devices were found: {}",
                ports.join(", ")
            ),
            Self::Transport(message) => write!(formatter, "transport error: {message}"),
            Self::Timeout(message) => write!(formatter, "timeout: {message}"),
            Self::Protocol(message) => write!(formatter, "protocol error: {message}"),
            Self::Identity(message) => write!(formatter, "identity error: {message}"),
            Self::Safety(message) => write!(formatter, "safety error: {message}"),
        }
    }
}

impl std::error::Error for NanoError {}

pub trait LineTransport: Send {
    fn write_line(&mut self, line: &str) -> io::Result<()>;
    fn read_line(&mut self, timeout: Duration) -> io::Result<Option<String>>;
}

struct SerialLineTransport {
    port: Box<dyn SerialPort>,
    receive_buffer: Vec<u8>,
}

impl SerialLineTransport {
    fn open(port_name: &str) -> Result<Self, NanoError> {
        let port = serialport::new(port_name, BAUD_RATE)
            .timeout(Duration::from_millis(50))
            .open()
            .map_err(|error| NanoError::Transport(error.to_string()))?;
        Ok(Self {
            port,
            receive_buffer: Vec::with_capacity(128),
        })
    }
}

impl LineTransport for SerialLineTransport {
    fn write_line(&mut self, line: &str) -> io::Result<()> {
        self.port.write_all(line.as_bytes())?;
        self.port.write_all(b"\n")?;
        self.port.flush()
    }

    fn read_line(&mut self, timeout: Duration) -> io::Result<Option<String>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(position) = self.receive_buffer.iter().position(|byte| *byte == b'\n') {
                let mut line = self.receive_buffer.drain(..=position).collect::<Vec<_>>();
                while matches!(line.last(), Some(b'\n' | b'\r')) {
                    line.pop();
                }
                return String::from_utf8(line)
                    .map(Some)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
            }
            if self.receive_buffer.len() > MAX_LINE_BYTES {
                self.receive_buffer.clear();
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "serial line exceeded 512 bytes",
                ));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            self.port
                .set_timeout((deadline - Instant::now()).min(Duration::from_millis(50)))
                .map_err(|error| io::Error::new(io::ErrorKind::Other, error.to_string()))?;
            let mut buffer = [0_u8; 64];
            match self.port.read(&mut buffer) {
                Ok(count) if count > 0 => self.receive_buffer.extend_from_slice(&buffer[..count]),
                Ok(_) => {}
                Err(error) if matches!(error.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) => {}
                Err(error) => return Err(error),
            }
        }
    }
}

struct Session {
    sender: mpsc::Sender<WorkerCommand>,
    emergency_sender: mpsc::Sender<mpsc::Sender<Result<(), NanoError>>>,
    worker: Option<JoinHandle<()>>,
    health: Arc<Mutex<NanoHealth>>,
    closed: Arc<AtomicBool>,
}

impl Session {
    fn call<T>(
        &self,
        make_command: impl FnOnce(mpsc::Sender<Result<T, NanoError>>) -> WorkerCommand,
        timeout: Duration,
    ) -> Result<T, NanoError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(NanoError::NotConnected);
        }
        let (sender, receiver) = mpsc::channel();
        self.sender
            .send(make_command(sender))
            .map_err(|_| NanoError::NotConnected)?;
        receiver
            .recv_timeout(timeout)
            .map_err(|_| NanoError::Timeout("adapter worker did not answer".into()))?
    }

    fn stop_best_effort(&self) {
        let _ = self.stop_emergency();
    }

    fn stop_emergency(&self) -> Result<(), NanoError> {
        let (sender, receiver) = mpsc::channel();
        self.emergency_sender
            .send(sender)
            .map_err(|_| NanoError::NotConnected)?;
        receiver
            .recv_timeout(STOP_TIMEOUT + Duration::from_millis(500))
            .map_err(|_| NanoError::Timeout("emergency STOP did not answer".into()))?
    }
}

pub struct NanoAdapter {
    session: Option<Session>,
    health: Arc<Mutex<NanoHealth>>,
}

impl Default for NanoAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl NanoAdapter {
    pub fn new() -> Self {
        let health = Arc::new(Mutex::new(NanoHealth::default()));
        Self {
            session: None,
            health,
        }
    }

    pub fn health(&self) -> NanoHealth {
        self.health.lock().expect("Nano health mutex poisoned").clone()
    }

    pub fn discover_and_connect(&mut self) -> Result<NanoIdentity, NanoError> {
        if self.session.is_some() {
            return Err(NanoError::Busy);
        }
        set_health(&self.health, |health| {
            health.state = NanoConnectionState::Discovering;
            health.last_error = None;
        });
        let mut ports = serialport::available_ports()
            .map_err(|error| NanoError::Transport(error.to_string()))?;
        ports.retain(is_ch340);
        ports.sort_by(|left, right| left.port_name.cmp(&right.port_name));
        let mut matches = Vec::new();
        let mut rejected = Vec::new();
        for port in ports {
            match SerialLineTransport::open(&port.port_name)
                .and_then(|transport| Self::probe_transport(port.port_name.clone(), Box::new(transport)))
            {
                Ok(candidate) => matches.push(candidate),
                Err(error) => rejected.push(format!("{}: {error}", port.port_name)),
            }
        }
        match matches.len() {
            0 => {
                let detail = if rejected.is_empty() {
                    "no serial ports were enumerated".into()
                } else {
                    rejected.join("; ")
                };
                set_health(&self.health, |health| {
                    health.state = NanoConnectionState::Disconnected;
                    health.last_error = Some(detail);
                });
                Err(NanoError::NoDevice)
            }
            1 => {
                let (port, identity, transport) = matches.pop().expect("one match");
                self.start_session(port, identity.clone(), transport)?;
                Ok(identity)
            }
            _ => {
                let names = matches.into_iter().map(|(port, _, _)| port).collect::<Vec<_>>();
                set_health(&self.health, |health| {
                    health.state = NanoConnectionState::Fault;
                    health.last_error = Some(format!("ambiguous devices: {}", names.join(", ")));
                });
                Err(NanoError::Ambiguous(names))
            }
        }
    }

    pub fn connect_transport_for_test(
        &mut self,
        port: impl Into<String>,
        transport: Box<dyn LineTransport>,
    ) -> Result<NanoIdentity, NanoError> {
        if self.session.is_some() {
            return Err(NanoError::Busy);
        }
        let (port, identity, transport) = Self::probe_transport(port.into(), transport)?;
        self.start_session(port, identity.clone(), transport)?;
        Ok(identity)
    }

    fn probe_transport(
        port: String,
        mut transport: Box<dyn LineTransport>,
    ) -> Result<(String, NanoIdentity, Box<dyn LineTransport>), NanoError> {
        let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
        let ready = read_optional_ready(&mut *transport, Duration::from_millis(350))?;
        let mut next_id = 1_u32;
        retry_ping(&mut *transport, &mut next_id, deadline)?;
        let info_id = allocate_id(&mut next_id);
        transport
            .write_line(&format!("INFO {info_id}"))
            .map_err(|error| NanoError::Transport(error.to_string()))?;
        let info_line = expect_ack_and(&mut *transport, info_id, "INFO", deadline, |line| {
            line.starts_with(&format!("INFO {info_id} "))
        })?;
        let identity = parse_identity(&info_line, Some(info_id))?;
        identity.require_supported()?;
        if let Some(ready) = ready {
            if ready != identity {
                return Err(NanoError::Identity(
                    "READY and INFO identities do not match".into(),
                ));
            }
        }
        let heartbeat_started = Instant::now();
        let heartbeat_deadline = heartbeat_started + HEARTBEAT_TIMEOUT;
        transport
            .write_line("HEARTBEAT 0")
            .map_err(|error| NanoError::Transport(error.to_string()))?;
        wait_for_line(&mut *transport, heartbeat_deadline, |line| line == "HBACK 0")?;
        if heartbeat_started.elapsed() > HEARTBEAT_TIMEOUT {
            return Err(NanoError::Timeout(
                "initial heartbeat acknowledgement was late".into(),
            ));
        }
        Ok((port, identity, transport))
    }

    fn start_session(
        &mut self,
        port: String,
        identity: NanoIdentity,
        transport: Box<dyn LineTransport>,
    ) -> Result<(), NanoError> {
        let health = self.health.clone();
        set_health(&health, |state| {
            state.state = NanoConnectionState::Connected;
            state.port = Some(port);
            state.identity = Some(identity);
            state.status = None;
            state.last_heartbeat_rtt_ms = Some(0);
            state.last_error = None;
        });
        let (sender, receiver) = mpsc::channel();
        let (emergency_sender, emergency_receiver) = mpsc::channel();
        let closed = Arc::new(AtomicBool::new(false));
        let worker_closed = closed.clone();
        let worker_health = health.clone();
        let worker = thread::Builder::new()
            .name("rts9060-nano".into())
            .spawn(move || {
                run_worker(
                    transport,
                    receiver,
                    emergency_receiver,
                    worker_health,
                    worker_closed,
                )
            })
            .map_err(|error| NanoError::Transport(error.to_string()))?;
        self.session = Some(Session {
            sender,
            emergency_sender,
            worker: Some(worker),
            health,
            closed,
        });
        Ok(())
    }

    fn session(&self) -> Result<&Session, NanoError> {
        self.session.as_ref().ok_or(NanoError::NotConnected)
    }

    pub fn ping(&self) -> Result<(), NanoError> {
        self.session()?.call(WorkerCommand::Ping, COMMAND_TIMEOUT)
    }

    pub fn status(&self) -> Result<NanoStatus, NanoError> {
        self.session()?.call(WorkerCommand::Status, COMMAND_TIMEOUT)
    }

    pub fn rearm(&self) -> Result<NanoStatus, NanoError> {
        self.session()?.call(WorkerCommand::Rearm, COMMAND_TIMEOUT)
    }

    pub fn clear_fault(&self) -> Result<NanoStatus, NanoError> {
        self.session()?.call(WorkerCommand::ClearFault, COMMAND_TIMEOUT * 2)
    }

    pub fn home(&self) -> Result<NanoStatus, NanoError> {
        self.session()?.call(WorkerCommand::Home, HOME_TIMEOUT + COMMAND_TIMEOUT)
    }

    pub fn move_abs(&self, angle_mdeg: i32) -> Result<MoveTicket, NanoError> {
        self.session()?.call(
            |reply| WorkerCommand::MoveAbs(angle_mdeg, reply),
            MOVE_TIMEOUT + COMMAND_TIMEOUT,
        )
    }

    pub fn capture_done(&self, command_id: u32) -> Result<NanoStatus, NanoError> {
        self.session()?.call(
            |reply| WorkerCommand::CaptureDone(command_id, reply),
            COMMAND_TIMEOUT * 2,
        )
    }

    pub fn stop(&self) -> Result<(), NanoError> {
        self.session()?.stop_emergency()
    }

    pub fn set_xray_warning(&self, enabled: bool) -> Result<(), NanoError> {
        self.session()?.call(
            |reply| WorkerCommand::XrayWarning(enabled, reply),
            COMMAND_TIMEOUT,
        )
    }

    pub fn disconnect(&mut self) {
        let Some(mut session) = self.session.take() else {
            return;
        };
        let _ = session.call(
            |reply| WorkerCommand::XrayWarning(false, reply),
            COMMAND_TIMEOUT,
        );
        session.stop_best_effort();
        let _ = session.sender.send(WorkerCommand::Shutdown);
        if let Some(worker) = session.worker.take() {
            let _ = worker.join();
        }
        set_health(&session.health, |health| {
            health.state = NanoConnectionState::Disconnected;
            health.status = None;
            health.last_heartbeat_rtt_ms = None;
        });
    }
}

impl Drop for NanoAdapter {
    fn drop(&mut self) {
        self.disconnect();
    }
}

enum WorkerCommand {
    Ping(mpsc::Sender<Result<(), NanoError>>),
    Status(mpsc::Sender<Result<NanoStatus, NanoError>>),
    Rearm(mpsc::Sender<Result<NanoStatus, NanoError>>),
    ClearFault(mpsc::Sender<Result<NanoStatus, NanoError>>),
    Home(mpsc::Sender<Result<NanoStatus, NanoError>>),
    MoveAbs(i32, mpsc::Sender<Result<MoveTicket, NanoError>>),
    CaptureDone(u32, mpsc::Sender<Result<NanoStatus, NanoError>>),
    XrayWarning(bool, mpsc::Sender<Result<(), NanoError>>),
    Shutdown,
}

struct Worker {
    transport: Box<dyn LineTransport>,
    receiver: mpsc::Receiver<WorkerCommand>,
    emergency_receiver: mpsc::Receiver<mpsc::Sender<Result<(), NanoError>>>,
    health: Arc<Mutex<NanoHealth>>,
    closed: Arc<AtomicBool>,
    next_id: u32,
    heartbeat_sequence: u32,
    next_heartbeat: Instant,
}

fn run_worker(
    transport: Box<dyn LineTransport>,
    receiver: mpsc::Receiver<WorkerCommand>,
    emergency_receiver: mpsc::Receiver<mpsc::Sender<Result<(), NanoError>>>,
    health: Arc<Mutex<NanoHealth>>,
    closed: Arc<AtomicBool>,
) {
    let mut worker = Worker {
        transport,
        receiver,
        emergency_receiver,
        health,
        closed,
        next_id: 1,
        heartbeat_sequence: 1,
        next_heartbeat: Instant::now() + HEARTBEAT_INTERVAL,
    };
    worker.run();
}

impl Worker {
    fn run(&mut self) {
        loop {
            if let Ok(reply) = self.emergency_receiver.try_recv() {
                let result = self.stop_protocol();
                let failed = result.is_err();
                let _ = reply.send(result);
                if failed {
                    self.fail(NanoError::Safety("emergency STOP failed".into()));
                    break;
                }
                continue;
            }
            let timeout = self
                .next_heartbeat
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(50));
            let command_error = match self.receiver.recv_timeout(timeout) {
                Ok(WorkerCommand::Ping(reply)) => reply_and_error(reply, self.ping()),
                Ok(WorkerCommand::Status(reply)) => reply_and_error(reply, self.status()),
                Ok(WorkerCommand::Rearm(reply)) => reply_and_error(reply, self.rearm()),
                Ok(WorkerCommand::ClearFault(reply)) => {
                    reply_and_error(reply, self.clear_fault())
                }
                Ok(WorkerCommand::Home(reply)) => reply_and_error(reply, self.home()),
                Ok(WorkerCommand::MoveAbs(angle_mdeg, reply)) => {
                    reply_and_error(reply, self.move_abs(angle_mdeg))
                }
                Ok(WorkerCommand::CaptureDone(command_id, reply)) => {
                    reply_and_error(reply, self.capture_done(command_id))
                }
                Ok(WorkerCommand::XrayWarning(enabled, reply)) => {
                    reply_and_error(reply, self.set_xray_warning(enabled))
                }
                Ok(WorkerCommand::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => None,
            };
            if let Some(error) = command_error {
                self.fail(error);
                break;
            }
            if Instant::now() >= self.next_heartbeat {
                // Only a transport-level failure (port gone) ends the session.
                // A late or missing HBACK is recorded and retried 500 ms
                // later: USB contention during camera transfers must not
                // permanently kill an otherwise healthy link.
                match self.heartbeat() {
                    Ok(()) => {}
                    Err(error @ NanoError::Transport(_)) => {
                        self.fail(error);
                        break;
                    }
                    Err(error) => {
                        set_health(&self.health, |health| {
                            health.last_error = Some(error.to_string());
                        });
                    }
                }
                self.next_heartbeat = Instant::now() + HEARTBEAT_INTERVAL;
            }
            if let Err(error) = self.poll_async_line() {
                if matches!(error, NanoError::Transport(_)) {
                    self.fail(error);
                    break;
                }
                set_health(&self.health, |health| {
                    health.last_error = Some(error.to_string());
                });
            }
        }
        self.closed.store(true, Ordering::SeqCst);
    }

    fn fail(&self, error: NanoError) {
        set_health(&self.health, |health| {
            health.state = NanoConnectionState::Lost;
            health.status = None;
            health.last_error = Some(error.to_string());
        });
    }

    fn command_id(&mut self) -> u32 {
        allocate_id(&mut self.next_id)
    }

    fn command_lines(
        &mut self,
        command: &str,
        arguments: &str,
        timeout: Duration,
        terminal: impl Fn(&str, u32) -> bool,
    ) -> Result<(u32, String), NanoError> {
        let id = self.command_id();
        self.command_lines_with_id(id, command, arguments, timeout, terminal)
    }

    fn command_lines_with_id(
        &mut self,
        id: u32,
        command: &str,
        arguments: &str,
        timeout: Duration,
        terminal: impl Fn(&str, u32) -> bool,
    ) -> Result<(u32, String), NanoError> {
        let request = if arguments.is_empty() {
            format!("{command} {id}")
        } else {
            format!("{command} {id} {arguments}")
        };
        self.transport
            .write_line(&request)
            .map_err(|error| NanoError::Transport(error.to_string()))?;
        let deadline = Instant::now() + timeout;
        let mut acknowledged = false;
        let mut pending_heartbeat: Option<(u32, Instant)> = None;
        loop {
            if let Ok(reply) = self.emergency_receiver.try_recv() {
                let stop_result = self.stop_protocol();
                let stop_confirmed = stop_result.is_ok();
                let _ = reply.send(stop_result);
                return Err(NanoError::Safety(if stop_confirmed {
                    format!("{command} interrupted by confirmed STOP")
                } else {
                    format!("{command} interrupted; STOP was not confirmed")
                }));
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(NanoError::Timeout(format!("{command} {id}")));
            }
            if let Some((sequence, started)) = pending_heartbeat {
                if now.duration_since(started) >= HEARTBEAT_TIMEOUT {
                    return Err(NanoError::Timeout(format!("missing HBACK {sequence}")));
                }
            } else if now >= self.next_heartbeat {
                let sequence = self.heartbeat_sequence;
                self.heartbeat_sequence = self.heartbeat_sequence.wrapping_add(1).max(1);
                self.transport
                    .write_line(&format!("HEARTBEAT {sequence}"))
                    .map_err(|error| NanoError::Transport(error.to_string()))?;
                pending_heartbeat = Some((sequence, now));
            }
            let line = self.read_line((deadline - Instant::now()).min(Duration::from_millis(100)))?;
            let Some(line) = line else {
                continue;
            };
            if let Some((sequence, started)) = pending_heartbeat {
                if line == format!("HBACK {sequence}") {
                    let rtt = started.elapsed();
                    set_health(&self.health, |health| {
                        health.last_heartbeat_rtt_ms = Some(rtt.as_millis() as u64)
                    });
                    pending_heartbeat = None;
                    self.next_heartbeat = Instant::now() + HEARTBEAT_INTERVAL;
                    continue;
                }
            }
            if line == format!("ACK {id} {command}") {
                acknowledged = true;
                continue;
            }
            if terminal(&line, id) {
                if !acknowledged {
                    return Err(NanoError::Protocol(format!(
                        "{command} terminal response arrived before ACK: {line}"
                    )));
                }
                return Ok((id, line));
            }
            if let Some(error) = command_failure(&line, id) {
                return Err(error);
            }
            self.handle_async(line)?;
        }
    }

    fn ping(&mut self) -> Result<(), NanoError> {
        self.command_lines("PING", "", COMMAND_TIMEOUT, |line, id| line == format!("PONG {id}"))?;
        Ok(())
    }

    fn status(&mut self) -> Result<NanoStatus, NanoError> {
        let (_, line) = self.command_lines("STATUS", "", COMMAND_TIMEOUT, |line, id| {
            line.starts_with(&format!("STATUS {id} "))
        })?;
        let status = parse_status(&line)?;
        set_health(&self.health, |health| health.status = Some(status.clone()));
        Ok(status)
    }

    fn rearm(&mut self) -> Result<NanoStatus, NanoError> {
        self.command_lines("REARM", "", COMMAND_TIMEOUT, |line, id| {
            line == format!("OK {id} REARMED")
        })?;
        let status = self.status()?;
        if !status.rearmed || !matches!(status.state.as_str(), "IDLE" | "LOCKED") {
            return Err(NanoError::Safety(
                "REARM did not produce an idle rearmed controller".into(),
            ));
        }
        Ok(status)
    }

    fn home(&mut self) -> Result<NanoStatus, NanoError> {
        self.command_lines("HOME", "", HOME_TIMEOUT, |line, id| {
            line == format!("HOME_DONE {id} POS=0")
        })?;
        let status = self.status()?;
        if status.state != "IDLE" || !status.rearmed || !status.homed || !status.reference_valid {
            return Err(NanoError::Safety(
                "HOME completed without a valid idle reference".into(),
            ));
        }
        Ok(status)
    }

    fn clear_fault(&mut self) -> Result<NanoStatus, NanoError> {
        self.command_lines("CLEAR_FAULT", "", COMMAND_TIMEOUT, |line, id| {
            line == format!("OK {id} FAULT_CLEARED")
        })?;
        let status = self.status()?;
        if status.state != "LOCKED"
            || status.rearmed
            || status.homed
            || status.reference_valid
        {
            return Err(NanoError::Safety(
                "CLEAR_FAULT did not restore a locked invalid-reference state".into(),
            ));
        }
        Ok(status)
    }

    fn move_abs(&mut self, angle_mdeg: i32) -> Result<MoveTicket, NanoError> {
        let (command_id, line) = self.command_lines(
            "MOVE_ABS",
            &angle_mdeg.to_string(),
            MOVE_TIMEOUT,
            |line, id| line.starts_with(&format!("READY_TO_CAPTURE {id} POS=")),
        )?;
        let position_pulses = line
            .split_once(" POS=")
            .and_then(|(_, value)| value.parse::<i64>().ok())
            .ok_or_else(|| NanoError::Protocol(format!("invalid READY_TO_CAPTURE: {line}")))?;
        let status = self.status()?;
        if status.state != "CAPTURE_HOLD"
            || status.capture_id != command_id
            || status.position_pulses != position_pulses
            || !status.rearmed
            || !status.homed
            || !status.reference_valid
        {
            return Err(NanoError::Safety(
                "MOVE_ABS did not end in a verified CAPTURE_HOLD".into(),
            ));
        }
        Ok(MoveTicket {
            command_id,
            angle_mdeg,
            position_pulses,
        })
    }

    fn capture_done(&mut self, command_id: u32) -> Result<NanoStatus, NanoError> {
        self.command_lines_with_id(
            command_id,
            "CAPTURE_DONE",
            "",
            COMMAND_TIMEOUT,
            |line, id| line == format!("IDLE {id} CAPTURE_RELEASED"),
        )?;
        let status = self.status()?;
        if status.state != "IDLE"
            || status.capture_id != 0
            || !status.rearmed
            || !status.homed
            || !status.reference_valid
        {
            return Err(NanoError::Safety(
                "CAPTURE_DONE did not restore a verified IDLE state".into(),
            ));
        }
        Ok(status)
    }

    fn stop_protocol(&mut self) -> Result<(), NanoError> {
        let result = self.command_lines("STOP", "", STOP_TIMEOUT, |line, id| {
            line.starts_with(&format!("STOPPED {id} "))
        });
        set_health(&self.health, |health| health.status = None);
        result.map(|_| ())
    }

    fn set_xray_warning(&mut self, enabled: bool) -> Result<(), NanoError> {
        let state = if enabled { "ON" } else { "OFF" };
        self.command_lines("XRAY_WARNING", state, COMMAND_TIMEOUT, |line, id| {
            line == format!("OK {id} XRAY_WARNING={state}")
        })?;
        Ok(())
    }

    fn heartbeat(&mut self) -> Result<(), NanoError> {
        let sequence = self.heartbeat_sequence;
        self.heartbeat_sequence = self.heartbeat_sequence.wrapping_add(1).max(1);
        let started = Instant::now();
        self.transport
            .write_line(&format!("HEARTBEAT {sequence}"))
            .map_err(|error| NanoError::Transport(error.to_string()))?;
        let deadline = started + HEARTBEAT_TIMEOUT;
        loop {
            let line = self.read_line((deadline - Instant::now()).min(Duration::from_millis(100)))?;
            let Some(line) = line else {
                if Instant::now() >= deadline {
                    return Err(NanoError::Timeout(format!(
                        "missing HBACK {sequence}"
                    )));
                }
                continue;
            };
            if line == format!("HBACK {sequence}") {
                let rtt = started.elapsed();
                if rtt > HEARTBEAT_TIMEOUT {
                    return Err(NanoError::Timeout(format!(
                        "late HBACK {sequence} after {} ms",
                        rtt.as_millis()
                    )));
                }
                set_health(&self.health, |health| {
                    health.last_heartbeat_rtt_ms = Some(rtt.as_millis() as u64)
                });
                return Ok(());
            }
            self.handle_async(line)?;
        }
    }

    fn poll_async_line(&mut self) -> Result<(), NanoError> {
        if let Some(line) = self.read_line(Duration::from_millis(1))? {
            self.handle_async(line)?;
        }
        Ok(())
    }

    fn read_line(&mut self, timeout: Duration) -> Result<Option<String>, NanoError> {
        self.transport
            .read_line(timeout)
            .map_err(|error| NanoError::Transport(error.to_string()))
    }

    fn handle_async(&mut self, line: String) -> Result<(), NanoError> {
        if line
            .strip_prefix("HBACK ")
            .is_some_and(|value| value.parse::<u32>().is_ok())
        {
            // A valid acknowledgement may arrive just after its bounded RTT
            // window and before the next command response. It must not satisfy
            // a newer heartbeat, but it is not a protocol violation either.
            return Ok(());
        }
        if line.starts_with("HOME_PHASE ") || line.starts_with("HOME_REQUEST ") {
            return Ok(());
        }
        if line.starts_with("FAULT ") || line.starts_with("STOPPED ") || line == "BUTTON_STOP" {
            set_health(&self.health, |health| {
                health.state = NanoConnectionState::Fault;
                health.status = None;
                health.last_error = Some(line.clone());
            });
            return Err(NanoError::Safety(line));
        }
        // Stale or unexpected lines (late responses from an already-failed
        // command, noise, out-of-order ERR for another id) are recorded but
        // never abort the in-flight command or the session.
        set_health(&self.health, |health| {
            health.last_error = Some(format!("ignored asynchronous line: {line}"));
        });
        Ok(())
    }
}

fn reply_and_error<T>(
    reply: mpsc::Sender<Result<T, NanoError>>,
    result: Result<T, NanoError>,
) -> Option<NanoError> {
    match result {
        Ok(value) => {
            let _ = reply.send(Ok(value));
            None
        }
        Err(error) => {
            // Command-level failures (device ERR responses, safety rejections,
            // per-command timeouts) are returned to the caller but keep the
            // session alive so recovery (STATUS/REARM/HOME) remains possible.
            // Only transport failures invalidate the session permanently.
            let fatal = matches!(error, NanoError::Transport(_));
            let _ = reply.send(Err(error));
            if fatal {
                Some(NanoError::Safety("session invalidated: transport failure".into()))
            } else {
                None
            }
        }
    }
}

fn set_health(health: &Arc<Mutex<NanoHealth>>, update: impl FnOnce(&mut NanoHealth)) {
    if let Ok(mut state) = health.lock() {
        update(&mut state);
    }
}

fn is_ch340(port: &SerialPortInfo) -> bool {
    matches!(
        &port.port_type,
        SerialPortType::UsbPort(usb) if CH340_IDS.contains(&(usb.vid, usb.pid))
    )
}

fn allocate_id(next_id: &mut u32) -> u32 {
    let id = (*next_id).max(1);
    *next_id = (*next_id).wrapping_add(1).max(1);
    id
}

fn read_optional_ready(
    transport: &mut dyn LineTransport,
    timeout: Duration,
) -> Result<Option<NanoIdentity>, NanoError> {
    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() >= deadline {
            return Ok(None);
        }
        match transport
            .read_line((deadline - Instant::now()).min(Duration::from_millis(50)))
            .map_err(|error| NanoError::Transport(error.to_string()))?
        {
            Some(line) if line.starts_with("READY ") => return parse_identity(&line, None).map(Some),
            Some(_) => {}
            None => {}
        }
    }
}

fn retry_ping(
    transport: &mut dyn LineTransport,
    next_id: &mut u32,
    deadline: Instant,
) -> Result<(), NanoError> {
    loop {
        if Instant::now() >= deadline {
            return Err(NanoError::Timeout("controller did not answer PING".into()));
        }
        let id = allocate_id(next_id);
        transport
            .write_line(&format!("PING {id}"))
            .map_err(|error| NanoError::Transport(error.to_string()))?;
        let attempt_deadline = (Instant::now() + Duration::from_millis(350)).min(deadline);
        let mut acknowledged = false;
        while Instant::now() < attempt_deadline {
            let Some(line) = transport
                .read_line((attempt_deadline - Instant::now()).min(Duration::from_millis(50)))
                .map_err(|error| NanoError::Transport(error.to_string()))?
            else {
                continue;
            };
            if line == format!("ACK {id} PING") {
                acknowledged = true;
            } else if line == format!("PONG {id}") && acknowledged {
                return Ok(());
            } else if let Some(error) = command_failure(&line, id) {
                return Err(error);
            }
        }
    }
}

fn expect_ack_and(
    transport: &mut dyn LineTransport,
    id: u32,
    command: &str,
    deadline: Instant,
    terminal: impl Fn(&str) -> bool,
) -> Result<String, NanoError> {
    let mut acknowledged = false;
    loop {
        if Instant::now() >= deadline {
            return Err(NanoError::Timeout(format!("{command} {id}")));
        }
        let Some(line) = transport
            .read_line((deadline - Instant::now()).min(Duration::from_millis(100)))
            .map_err(|error| NanoError::Transport(error.to_string()))?
        else {
            continue;
        };
        if line == format!("ACK {id} {command}") {
            acknowledged = true;
        } else if let Some(error) = command_failure(&line, id) {
            return Err(error);
        } else if terminal(&line) {
            if !acknowledged {
                return Err(NanoError::Protocol(format!(
                    "{command} terminal response arrived before ACK: {line}"
                )));
            }
            return Ok(line);
        }
    }
}

fn wait_for_line(
    transport: &mut dyn LineTransport,
    deadline: Instant,
    predicate: impl Fn(&str) -> bool,
) -> Result<String, NanoError> {
    loop {
        if Instant::now() >= deadline {
            return Err(NanoError::Timeout("expected response was not received".into()));
        }
        if let Some(line) = transport
            .read_line((deadline - Instant::now()).min(Duration::from_millis(100)))
            .map_err(|error| NanoError::Transport(error.to_string()))?
        {
            if line.starts_with("ERR ") || line.starts_with("FAULT ") || line.starts_with("STOPPED ") {
                return Err(NanoError::Protocol(line));
            }
            if predicate(&line) {
                return Ok(line);
            }
        }
    }
}

fn command_failure(line: &str, id: u32) -> Option<NanoError> {
    if line.starts_with(&format!("ERR {id} ")) {
        Some(NanoError::Protocol(line.into()))
    } else if line.starts_with("ERR ") {
        Some(NanoError::Protocol(line.into()))
    } else if line.starts_with("FAULT ") || line.starts_with("STOPPED ") || line == "BUTTON_STOP" {
        Some(NanoError::Safety(line.into()))
    } else {
        None
    }
}

fn parse_identity(line: &str, info_id: Option<u32>) -> Result<NanoIdentity, NanoError> {
    let tokens = line.split_whitespace().collect::<Vec<_>>();
    let start = match info_id {
        Some(id)
            if tokens.first() == Some(&"INFO")
                && tokens
                    .get(1)
                    .and_then(|token| token.parse::<u32>().ok())
                    == Some(id) =>
        {
            2
        }
        None if tokens.first() == Some(&"READY") => 1,
        _ => return Err(NanoError::Protocol(format!("invalid identity line: {line}"))),
    };
    let fields = parse_fields(&tokens[start..])?;
    let capabilities = required_field(&fields, "CAPS")?
        .split(',')
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect();
    Ok(NanoIdentity {
        device: required_field(&fields, "DEVICE")?.to_owned(),
        version: required_field(&fields, "VERSION")?.to_owned(),
        protocol: parse_field(&fields, "PROTOCOL")?,
        build: required_field(&fields, "BUILD")?.to_owned(),
        buzzer_installed: match required_field(&fields, "BUZZER")? {
            "1" => true,
            "0" => false,
            value => return Err(NanoError::Protocol(format!("invalid BUZZER value {value}"))),
        },
        capabilities,
    })
}

fn parse_status(line: &str) -> Result<NanoStatus, NanoError> {
    let tokens = line.split_whitespace().collect::<Vec<_>>();
    if tokens.first() != Some(&"STATUS") || tokens.len() < 3 {
        return Err(NanoError::Protocol(format!("invalid STATUS line: {line}")));
    }
    let fields = parse_fields(&tokens[2..])?;
    Ok(NanoStatus {
        state: required_field(&fields, "state")?.to_owned(),
        position_pulses: parse_field(&fields, "pos")?,
        target_pulses: parse_field(&fields, "target")?,
        microsteps: parse_field(&fields, "microsteps")?,
        pulses_per_rev: parse_field(&fields, "ppr")?,
        reference_valid: parse_bool_field(&fields, "reference")?,
        homed: parse_bool_field(&fields, "homed")?,
        rearmed: parse_bool_field(&fields, "rearmed")?,
        hall_active: parse_bool_field(&fields, "hall")?,
        capture_id: parse_field(&fields, "capture_id")?,
    })
}

fn parse_fields<'a>(tokens: &'a [&'a str]) -> Result<HashMap<&'a str, &'a str>, NanoError> {
    let mut fields = HashMap::new();
    for token in tokens {
        let (key, value) = token
            .split_once('=')
            .ok_or_else(|| NanoError::Protocol(format!("invalid field {token}")))?;
        if key.is_empty() || value.is_empty() || fields.insert(key, value).is_some() {
            return Err(NanoError::Protocol(format!("invalid duplicate field {token}")));
        }
    }
    Ok(fields)
}

fn required_field<'a>(fields: &'a HashMap<&str, &str>, key: &str) -> Result<&'a str, NanoError> {
    fields
        .get(key)
        .copied()
        .ok_or_else(|| NanoError::Protocol(format!("missing field {key}")))
}

fn parse_field<T>(fields: &HashMap<&str, &str>, key: &str) -> Result<T, NanoError>
where
    T: std::str::FromStr,
{
    required_field(fields, key)?
        .parse()
        .map_err(|_| NanoError::Protocol(format!("invalid numeric field {key}")))
}

fn parse_bool_field(fields: &HashMap<&str, &str>, key: &str) -> Result<bool, NanoError> {
    match required_field(fields, key)? {
        "1" => Ok(true),
        "0" => Ok(false),
        _ => Err(NanoError::Protocol(format!("invalid boolean field {key}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct FakeState {
        queue: VecDeque<String>,
        writes: Vec<String>,
        rearmed: bool,
        homed: bool,
        drop_heartbeats: bool,
        wrong_device: bool,
        delay_home_until_heartbeat: bool,
        pending_home_id: Option<String>,
        position_pulses: i64,
        capture_id: u32,
        hold_move: bool,
    }

    struct FakeTransport {
        state: Arc<Mutex<FakeState>>,
    }

    impl FakeTransport {
        fn new(state: Arc<Mutex<FakeState>>) -> Self {
            state.lock().unwrap().queue.push_back(identity_line("READY", false));
            Self { state }
        }
    }

    impl LineTransport for FakeTransport {
        fn write_line(&mut self, line: &str) -> io::Result<()> {
            let mut state = self.state.lock().unwrap();
            state.writes.push(line.into());
            let parts = line.split_whitespace().collect::<Vec<_>>();
            match parts.as_slice() {
                ["PING", id] => {
                    state.queue.push_back(format!("ACK {id} PING"));
                    state.queue.push_back(format!("PONG {id}"));
                }
                ["INFO", id] => {
                    let wrong_device = state.wrong_device;
                    state.queue.push_back(format!("ACK {id} INFO"));
                    state
                        .queue
                        .push_back(identity_line(&format!("INFO {id}"), wrong_device));
                }
                ["HEARTBEAT", sequence] if !state.drop_heartbeats => {
                    state.queue.push_back(format!("HBACK {sequence}"));
                    if let Some(id) = state.pending_home_id.take() {
                        state.homed = true;
                        state.queue.push_back(format!("HOME_DONE {id} POS=0"));
                    }
                }
                ["STATUS", id] => {
                    let rearmed = state.rearmed;
                    let homed = state.homed;
                    let position = state.position_pulses;
                    let capture_id = state.capture_id;
                    state.queue.push_back(format!("ACK {id} STATUS"));
                    state.queue.push_back(status_line(
                        id,
                        rearmed,
                        homed,
                        position,
                        capture_id,
                    ));
                }
                ["REARM", id] => {
                    state.rearmed = true;
                    state.queue.push_back(format!("ACK {id} REARM"));
                    state.queue.push_back(format!("OK {id} REARMED"));
                }
                ["HOME", id] => {
                    if !state.rearmed {
                        state.queue.push_back(format!("ERR {id} NOT_REARMED"));
                    } else {
                        state.queue.push_back(format!("ACK {id} HOME"));
                        state.queue.push_back("HOME_PHASE HOME_SEARCH_FAST limit=120000".into());
                        if state.delay_home_until_heartbeat {
                            state.pending_home_id = Some((*id).to_owned());
                        } else {
                            state.homed = true;
                            state.queue.push_back(format!("HOME_DONE {id} POS=0"));
                        }
                    }
                }
                ["XRAY_WARNING", id, state_value] => {
                    state.queue.push_back(format!("ACK {id} XRAY_WARNING"));
                    state
                        .queue
                        .push_back(format!("OK {id} XRAY_WARNING={state_value}"));
                }
                ["CLEAR_FAULT", id] => {
                    state.rearmed = false;
                    state.homed = false;
                    state.capture_id = 0;
                    state.queue.push_back(format!("ACK {id} CLEAR_FAULT"));
                    state.queue.push_back(format!("OK {id} FAULT_CLEARED"));
                }
                ["MOVE_ABS", id, angle] => {
                    if !state.rearmed || !state.homed {
                        state.queue.push_back(format!("ERR {id} REFERENCE_REQUIRED"));
                    } else {
                        let angle = angle.parse::<i64>().unwrap();
                        state.position_pulses = angle * 96_000 / 360_000;
                        state.capture_id = id.parse().unwrap();
                        let position = state.position_pulses;
                        state.queue.push_back(format!("ACK {id} MOVE_ABS"));
                        if !state.hold_move {
                            state.queue.push_back(format!(
                                "READY_TO_CAPTURE {id} POS={position}"
                            ));
                        }
                    }
                }
                ["CAPTURE_DONE", id] => {
                    if state.capture_id != id.parse::<u32>().unwrap() {
                        state
                            .queue
                            .push_back(format!("ERR {id} CAPTURE_ID_MISMATCH"));
                    } else {
                        state.capture_id = 0;
                        state.queue.push_back(format!("ACK {id} CAPTURE_DONE"));
                        state
                            .queue
                            .push_back(format!("IDLE {id} CAPTURE_RELEASED"));
                    }
                }
                ["STOP", id] => {
                    state.rearmed = false;
                    state.homed = false;
                    state.queue.push_back(format!("ACK {id} STOP"));
                    state.queue.push_back(format!(
                        "STOPPED {id} POSITION_UNKNOWN reason=HOST_STOP"
                    ));
                }
                _ => {}
            }
            Ok(())
        }

        fn read_line(&mut self, timeout: Duration) -> io::Result<Option<String>> {
            let deadline = Instant::now() + timeout;
            loop {
                if let Some(line) = self.state.lock().unwrap().queue.pop_front() {
                    return Ok(Some(line));
                }
                if Instant::now() >= deadline {
                    return Ok(None);
                }
                thread::sleep(Duration::from_millis(1));
            }
        }
    }

    fn identity_line(prefix: &str, wrong_device: bool) -> String {
        format!(
            "{prefix} DEVICE={} VERSION=2.1.0 PROTOCOL=2 BUILD=20260918 BUZZER=1 CAPS=HEARTBEAT_ACK,XRAY_WARNING",
            if wrong_device { "OTHER_DEVICE" } else { EXPECTED_DEVICE }
        )
    }

    fn status_line(
        id: &str,
        rearmed: bool,
        homed: bool,
        position_pulses: i64,
        capture_id: u32,
    ) -> String {
        format!(
            "STATUS {id} state={} pos={position_pulses} target={position_pulses} microsteps=8 ppr=96000 reference={} homed={} rearmed={} hall=0 capture_id={capture_id}",
            if capture_id != 0 { "CAPTURE_HOLD" } else if rearmed { "IDLE" } else { "LOCKED" },
            u8::from(homed),
            u8::from(homed),
            u8::from(rearmed)
        )
    }

    #[test]
    fn identity_requires_device_and_all_capabilities() {
        let identity = parse_identity(&identity_line("READY", false), None).unwrap();
        identity.require_supported().unwrap();
        assert_eq!(identity.device, EXPECTED_DEVICE);
        assert_eq!(identity.version, "2.1.0");
        assert_eq!(identity.protocol, 2);
    }

    #[test]
    fn wrong_device_is_rejected_before_session_start() {
        let state = Arc::new(Mutex::new(FakeState {
            wrong_device: true,
            ..FakeState::default()
        }));
        let transport = FakeTransport::new(state);
        let mut adapter = NanoAdapter::new();
        let error = adapter
            .connect_transport_for_test("COM-TEST", Box::new(transport))
            .unwrap_err();
        assert!(matches!(error, NanoError::Identity(_)));
        assert_eq!(adapter.health().state, NanoConnectionState::Disconnected);
    }

    #[test]
    fn rearm_home_status_and_stop_are_verified() {
        let state = Arc::new(Mutex::new(FakeState::default()));
        let transport = FakeTransport::new(state.clone());
        let mut adapter = NanoAdapter::new();
        adapter
            .connect_transport_for_test("COM-TEST", Box::new(transport))
            .unwrap();
        assert!(!adapter.status().unwrap().rearmed);
        assert!(adapter.rearm().unwrap().rearmed);
        let homed = adapter.home().unwrap();
        assert!(homed.homed && homed.reference_valid && homed.rearmed);
        adapter.set_xray_warning(true).unwrap();
        adapter.set_xray_warning(false).unwrap();
        adapter.stop().unwrap();
        assert!(adapter.health().status.is_none());
        adapter.disconnect();
        let writes = &state.lock().unwrap().writes;
        assert!(writes.iter().any(|line| line.starts_with("REARM ")));
        assert!(writes.iter().any(|line| line.starts_with("HOME ")));
        assert!(writes
            .iter()
            .any(|line| line.starts_with("XRAY_WARNING ") && line.ends_with(" ON")));
        assert!(writes
            .iter()
            .any(|line| line.starts_with("XRAY_WARNING ") && line.ends_with(" OFF")));
        assert!(writes.iter().any(|line| line.starts_with("STOP ")));
    }

    #[test]
    fn home_terminal_response_is_not_lost_when_interleaved_with_heartbeat() {
        let state = Arc::new(Mutex::new(FakeState {
            delay_home_until_heartbeat: true,
            ..FakeState::default()
        }));
        let transport = FakeTransport::new(state.clone());
        let mut adapter = NanoAdapter::new();
        adapter
            .connect_transport_for_test("COM-TEST", Box::new(transport))
            .unwrap();
        adapter.rearm().unwrap();
        thread::sleep(HEARTBEAT_INTERVAL + Duration::from_millis(50));
        let status = adapter.home().unwrap();
        assert!(status.homed && status.reference_valid && status.rearmed);
        let heartbeat_was_sent = state
            .lock()
            .unwrap()
            .writes
            .iter()
            .any(|line| line.starts_with("HEARTBEAT "));
        assert!(heartbeat_was_sent);
        adapter.disconnect();
    }

    #[test]
    fn heartbeat_loss_is_tolerated_and_the_link_recovers() {
        let state = Arc::new(Mutex::new(FakeState::default()));
        let transport = FakeTransport::new(state.clone());
        let mut adapter = NanoAdapter::new();
        adapter
            .connect_transport_for_test("COM-TEST", Box::new(transport))
            .unwrap();
        adapter.status().unwrap();
        state.lock().unwrap().drop_heartbeats = true;
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            let mentions_hback = adapter
                .health()
                .last_error
                .as_deref()
                .is_some_and(|error| error.contains("HBACK"));
            if mentions_hback {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let health = adapter.health();
        // A lost HBACK is recorded but no longer kills the session.
        assert_eq!(health.state, NanoConnectionState::Connected);
        assert!(health.last_error.unwrap().contains("HBACK"));
        state.lock().unwrap().drop_heartbeats = false;
        adapter.status().unwrap();
        assert_eq!(adapter.health().state, NanoConnectionState::Connected);
        adapter.disconnect();
    }

    #[test]
    fn a_rejected_command_keeps_the_session_alive_for_recovery() {
        let state = Arc::new(Mutex::new(FakeState::default()));
        let transport = FakeTransport::new(state.clone());
        let mut adapter = NanoAdapter::new();
        adapter
            .connect_transport_for_test("COM-TEST", Box::new(transport))
            .unwrap();
        // Device rejects CAPTURE_DONE outside CAPTURE_HOLD: the error is
        // returned to the caller but the session must survive.
        let error = adapter.capture_done(4_242).unwrap_err();
        assert!(matches!(error, NanoError::Safety(_) | NanoError::Protocol(_)));
        assert_eq!(adapter.health().state, NanoConnectionState::Connected);
        // Full recovery path still works on the same session.
        adapter.rearm().unwrap();
        adapter.home().unwrap();
        let ticket = adapter.move_abs(-72_000).unwrap();
        adapter.capture_done(ticket.command_id).unwrap();
        adapter.disconnect();
    }

    #[test]
    fn disconnect_attempts_warning_off_then_stop_before_shutdown() {
        let state = Arc::new(Mutex::new(FakeState::default()));
        let transport = FakeTransport::new(state.clone());
        let mut adapter = NanoAdapter::new();
        adapter
            .connect_transport_for_test("COM-TEST", Box::new(transport))
            .unwrap();
        adapter.disconnect();
        let writes = &state.lock().unwrap().writes;
        let warning_off = writes
            .iter()
            .position(|line| line.starts_with("XRAY_WARNING ") && line.ends_with(" OFF"))
            .unwrap();
        let stop = writes
            .iter()
            .position(|line| line.starts_with("STOP "))
            .unwrap();
        assert!(warning_off < stop);
    }

    #[test]
    fn move_and_capture_done_preserve_the_firmware_transaction_id() {
        let state = Arc::new(Mutex::new(FakeState::default()));
        let transport = FakeTransport::new(state.clone());
        let mut adapter = NanoAdapter::new();
        adapter
            .connect_transport_for_test("COM-TEST", Box::new(transport))
            .unwrap();
        adapter.rearm().unwrap();
        adapter.home().unwrap();
        let ticket = adapter.move_abs(-3_000).unwrap();
        assert_eq!(ticket.position_pulses, -800);
        let released = adapter.capture_done(ticket.command_id).unwrap();
        assert_eq!(released.state, "IDLE");
        let writes = state.lock().unwrap().writes.clone();
        assert!(writes
            .iter()
            .any(|line| line == &format!("CAPTURE_DONE {}", ticket.command_id)));
        adapter.disconnect();
    }

    #[test]
    fn emergency_stop_preempts_a_pending_move() {
        let state = Arc::new(Mutex::new(FakeState {
            hold_move: true,
            ..FakeState::default()
        }));
        let transport = FakeTransport::new(state.clone());
        let mut adapter = NanoAdapter::new();
        adapter
            .connect_transport_for_test("COM-TEST", Box::new(transport))
            .unwrap();
        adapter.rearm().unwrap();
        adapter.home().unwrap();
        let adapter = Arc::new(adapter);
        let moving = adapter.clone();
        let worker = thread::spawn(move || moving.move_abs(-90_000));
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline
            && !state
                .lock()
                .unwrap()
                .writes
                .iter()
                .any(|line| line.starts_with("MOVE_ABS "))
        {
            thread::sleep(Duration::from_millis(1));
        }
        adapter.stop().unwrap();
        assert!(matches!(worker.join().unwrap(), Err(NanoError::Safety(_))));
        drop(adapter);
    }
}
