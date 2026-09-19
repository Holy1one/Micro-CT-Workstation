use serde::Serialize;
use serialport::{ClearBuffer, SerialPort, SerialPortInfo, SerialPortType};
use std::fmt;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const MOXTEK_VID: u16 = 0x277B;
const MOXTEK_PID: u16 = 0x07D1;
const MOXTEK_SERIAL: &str = "168249";
const BAUD_RATE: u32 = 57_600;
const IO_TIMEOUT: Duration = Duration::from_secs(1);
const PACKET_HEADER: u8 = 0x1B;
const CMD_SET_VOLTAGE: u8 = 0x40;
const CMD_SET_CURRENT: u8 = 0x41;
const CMD_SET_XRAY_ENABLE: u8 = 0x42;
const CMD_GET_SETPOINTS: u8 = 0x43;
const CMD_GET_STATUS: u8 = 0x80;
// Device-side USB auto-shutdown (deadman) configuration, recovered from the
// manufacturer's 12WattController: 0x74 arms/releases the deadman, 0x76 sets
// its delay, 0x77 reads the configured delay back.
const CMD_USB_AUTO_SHUTDOWN: u8 = 0x74;
const CMD_USB_SHUTDOWN_DELAY: u8 = 0x76;
const CMD_GET_USB_SHUTDOWN_TIMER: u8 = 0x77;
const DAC_SCALE: f64 = 13_107.2;
const VOLTAGE_SCALE: f64 = 15.0;
const CURRENT_SCALE: f64 = 250.0;
const TEMP_OFFSET: f64 = 98.3;
const TEMP_SCALE: f64 = 3.19;
pub const MIN_VOLTAGE_KV: f64 = 4.0;
pub const MAX_VOLTAGE_KV: f64 = 70.0;
pub const MAX_CURRENT_UA: f64 = 1_000.0;
pub const MAX_SETPOINT_POWER_W: f64 = 12.0;
const VOLTAGE_READBACK_TOLERANCE_KV: f64 = 0.1;
const CURRENT_READBACK_TOLERANCE_UA: f64 = 1.0;
const MIN_REENABLE_INTERVAL: Duration = Duration::from_secs(2);
const EMISSION_STABILITY_TIMEOUT: Duration = Duration::from_secs(8);
const EMISSION_POLL_INTERVAL: Duration = Duration::from_millis(50);
const MAX_CASE_TEMPERATURE_C: f64 = 65.0;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XrayHealth {
    pub connected: bool,
    pub port: Option<String>,
    pub serial: Option<String>,
    pub beam_off_confirmed: bool,
    pub beam_on: bool,
    pub locked: Option<bool>,
    pub usb_auto_shutdown: Option<bool>,
    pub usb_shutdown_delay: Option<u32>,
    pub set_voltage_kv: Option<f64>,
    pub set_current_ua: Option<f64>,
    pub voltage_kv: Option<f64>,
    pub current_ua: Option<f64>,
    pub temperature_c: Option<f64>,
    pub last_error: Option<String>,
}

impl Default for XrayHealth {
    fn default() -> Self {
        Self {
            connected: false,
            port: None,
            serial: None,
            beam_off_confirmed: false,
            beam_on: false,
            locked: None,
            usb_auto_shutdown: None,
            usb_shutdown_delay: None,
            set_voltage_kv: None,
            set_current_ua: None,
            voltage_kv: None,
            current_ua: None,
            temperature_c: None,
            last_error: None,
        }
    }
}

#[derive(Debug)]
pub enum XrayError {
    Discovery(String),
    Transport(String),
    Protocol(String),
    Parameter(String),
    Safety(String),
}

impl fmt::Display for XrayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Discovery(message) => write!(f, "discovery error: {message}"),
            Self::Transport(message) => write!(f, "transport error: {message}"),
            Self::Protocol(message) => write!(f, "protocol error: {message}"),
            Self::Parameter(message) => write!(f, "parameter error: {message}"),
            Self::Safety(message) => write!(f, "safety error: {message}"),
        }
    }
}

impl std::error::Error for XrayError {}

trait BinaryTransport: Send {
    fn clear_input(&mut self) -> io::Result<()>;
    fn write_all_bytes(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn read_exact_bytes(&mut self, bytes: &mut [u8]) -> io::Result<()>;
}

struct SerialBinaryTransport {
    port: Box<dyn SerialPort>,
}

impl BinaryTransport for SerialBinaryTransport {
    fn clear_input(&mut self) -> io::Result<()> {
        self.port
            .clear(ClearBuffer::Input)
            .map_err(|error| io::Error::new(io::ErrorKind::Other, error.to_string()))
    }

    fn write_all_bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.port.write_all(bytes)
    }

    fn read_exact_bytes(&mut self, bytes: &mut [u8]) -> io::Result<()> {
        self.port.read_exact(bytes)
    }
}

#[derive(Debug)]
struct Setpoints {
    voltage_kv: f64,
    current_ua: f64,
    enabled: bool,
}

#[derive(Debug)]
struct Status {
    voltage_kv: f64,
    current_ua: f64,
    temperature_c: f64,
    locked: bool,
    kv_enabled: bool,
    ua_enabled: bool,
}

pub struct MoxtekAdapter {
    transport: Option<Box<dyn BinaryTransport>>,
    health: XrayHealth,
    requested_setpoint: Option<(f64, f64)>,
    last_off: Option<Instant>,
}

impl Default for MoxtekAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl MoxtekAdapter {
    pub fn new() -> Self {
        Self {
            transport: None,
            health: XrayHealth::default(),
            requested_setpoint: None,
            last_off: None,
        }
    }

    pub fn health(&self) -> XrayHealth {
        self.health.clone()
    }

    pub fn discover_and_connect(&mut self) -> Result<XrayHealth, XrayError> {
        if self.transport.is_some() {
            return Err(XrayError::Safety("Moxtek adapter is already connected".into()));
        }
        let ports = serialport::available_ports()
            .map_err(|error| XrayError::Discovery(error.to_string()))?;
        let mut matches: Vec<SerialPortInfo> = ports.into_iter().filter(is_exact_moxtek).collect();
        matches.sort_by(|left, right| left.port_name.cmp(&right.port_name));
        if matches.len() != 1 {
            return Err(XrayError::Discovery(format!(
                "expected exactly one connected Moxtek VID=277B PID=07D1 SN={MOXTEK_SERIAL}; found {}",
                matches.len()
            )));
        }
        let port_name = matches.remove(0).port_name;
        let port = serialport::new(&port_name, BAUD_RATE)
            .data_bits(serialport::DataBits::Eight)
            .parity(serialport::Parity::None)
            .stop_bits(serialport::StopBits::One)
            .flow_control(serialport::FlowControl::None)
            .timeout(IO_TIMEOUT)
            .open()
            .map_err(|error| XrayError::Transport(format!("could not open {port_name}: {error}")))?;
        self.connect_transport(
            port_name,
            MOXTEK_SERIAL.to_owned(),
            Box::new(SerialBinaryTransport { port }),
        )?;
        Ok(self.health())
    }

    /// Send an explicit OFF command and require both command and measured
    /// output flags to report OFF. Measured kV/uA are telemetry only here:
    /// calibration offsets must not prevent an emergency shutdown.
    pub fn force_off(&mut self) -> Result<XrayHealth, XrayError> {
        let result = self.force_off_inner();
        if let Err(error) = &result {
            self.health.beam_off_confirmed = false;
            self.health.last_error = Some(error.to_string());
        }
        result.map(|()| self.health())
    }

    /// Configure a complete operating point while the source remains OFF.
    /// Requested values, rather than noisy measured telemetry, carry the hard
    /// 4..70 kV, 0..1000 uA and 12 W limits.
    pub fn set_parameters(
        &mut self,
        voltage_kv: f64,
        current_ua: f64,
    ) -> Result<XrayHealth, XrayError> {
        validate_setpoint(voltage_kv, current_ua)?;
        if self.transport.is_none() {
            return Err(XrayError::Transport("Moxtek adapter is not connected".into()));
        }

        // Reconfirm OFF before any parameter transition. The current is sent
        // to zero first so persisted settings cannot create an over-power
        // intermediate point.
        self.force_off_inner()?;
        let existing = self.read_setpoints()?;
        if !within_setpoint_tolerance(&existing, voltage_kv, current_ua) {
            self.write_setpoint(CMD_SET_CURRENT, ua_to_raw(0.0)?)?;
            self.verify_one_setpoint(None, Some(0.0))?;
            self.write_setpoint(CMD_SET_VOLTAGE, kv_to_raw(voltage_kv)?)?;
            self.verify_one_setpoint(Some(voltage_kv), Some(0.0))?;
            self.write_setpoint(CMD_SET_CURRENT, ua_to_raw(current_ua)?)?;
        }

        let final_setpoints = self.read_setpoints()?;
        if final_setpoints.enabled {
            return self.fail(XrayError::Safety(
                "parameter write unexpectedly left the beam command enabled".into(),
            ));
        }
        if !within_setpoint_tolerance(&final_setpoints, voltage_kv, current_ua) {
            return self.fail(XrayError::Protocol(format!(
                "setpoint readback {:.4} kV / {:.4} uA differs from requested {:.4} kV / {:.4} uA",
                final_setpoints.voltage_kv,
                final_setpoints.current_ua,
                voltage_kv,
                current_ua
            )));
        }
        let status = self.read_status()?;
        self.require_off(&final_setpoints, &status)?;
        self.apply_health(&final_setpoints, &status);
        self.requested_setpoint = Some((voltage_kv, current_ua));
        Ok(self.health())
    }

    pub fn beam_on(&mut self, cancel: &AtomicBool) -> Result<XrayHealth, XrayError> {
        let result = self.beam_on_inner(cancel);
        if let Err(error) = &result {
            let _ = self.force_off_inner();
            self.health.beam_off_confirmed = false;
            self.health.last_error = Some(error.to_string());
        }
        result.map(|()| self.health())
    }

    pub fn refresh_off_status(&mut self) -> Result<XrayHealth, XrayError> {
        let setpoints = self.read_setpoints()?;
        let status = self.read_status()?;
        self.require_off(&setpoints, &status)?;
        self.apply_health(&setpoints, &status);
        Ok(self.health())
    }

    /// Non-destructive status poll: reads the live beam state and telemetry
    /// without commanding anything. A detected ON beam is reported, never
    /// silently cleared; transport failures mark the link disconnected.
    pub fn refresh_status(&mut self) -> Result<XrayHealth, XrayError> {
        let result: Result<(Setpoints, Status), XrayError> = (|| {
            let setpoints = self.read_setpoints()?;
            let status = self.read_status()?;
            Ok((setpoints, status))
        })();
        match result {
            Ok((setpoints, status)) => {
                let emitting = setpoints.enabled || status.kv_enabled || status.ua_enabled;
                self.health.connected = true;
                self.health.beam_on = emitting;
                self.health.beam_off_confirmed = !emitting;
                self.health.locked = Some(status.locked);
                self.health.set_voltage_kv = Some(setpoints.voltage_kv);
                self.health.set_current_ua = Some(setpoints.current_ua);
                self.health.voltage_kv = Some(status.voltage_kv);
                self.health.current_ua = Some(status.current_ua);
                self.health.temperature_c = Some(status.temperature_c);
                self.health.last_error = None;
                Ok(self.health())
            }
            Err(error) => {
                self.health.connected = false;
                self.health.beam_off_confirmed = false;
                self.health.last_error = Some(error.to_string());
                Err(error)
            }
        }
    }

    /// Arms or releases the device-side USB auto-shutdown deadman
    /// (manufacturer command 0x74). Armed: the tube kills its own output
    /// within the configured delay once the host stops talking — the hardware
    /// fail-safe if this program dies. Released: output persists under host
    /// control alone, so beam state is entirely the operator's
    /// responsibility.
    pub fn set_usb_auto_shutdown(&mut self, enabled: bool) -> Result<XrayHealth, XrayError> {
        self.transact(CMD_USB_AUTO_SHUTDOWN, &[u8::from(enabled)], 4)?;
        self.health.usb_auto_shutdown = Some(enabled);
        Ok(self.health())
    }

    /// Reads the configured USB shutdown delay (0x77). Firmware above
    /// version 6 answers with a u16; older firmware answers with one byte.
    pub fn read_usb_shutdown_timer(&mut self) -> Result<u32, XrayError> {
        match self.transact(CMD_GET_USB_SHUTDOWN_TIMER, &[], 5) {
            Ok(response) => {
                let value = u32::from(read_u16(&response, 3)?);
                self.health.usb_shutdown_delay = Some(value);
                Ok(value)
            }
            Err(first_error) => {
                let response = self
                    .transact(CMD_GET_USB_SHUTDOWN_TIMER, &[], 4)
                    .map_err(|_| first_error)?;
                let value = u32::from(response[3]);
                self.health.usb_shutdown_delay = Some(value);
                Ok(value)
            }
        }
    }

    /// Writes the USB shutdown delay (0x76), mirroring the manufacturer
    /// controller's "Shut Down Delay / SET": u16 little-endian payload on
    /// firmware above version 6, a single byte (capped at 255) on older
    /// firmware. The written value is read back into health.
    pub fn set_usb_shutdown_delay(&mut self, delay: u16) -> Result<XrayHealth, XrayError> {
        let payload = delay.to_le_bytes();
        match self.transact(CMD_USB_SHUTDOWN_DELAY, &payload, 5) {
            Ok(_) => {}
            Err(first_error) => {
                let byte = u8::try_from(delay).unwrap_or(u8::MAX);
                self.transact(CMD_USB_SHUTDOWN_DELAY, &[byte], 4)
                    .map_err(|_| first_error)?;
            }
        }
        let readback = self.read_usb_shutdown_timer()?;
        let capped = u32::from(delay).min(255);
        if readback != u32::from(delay) && readback != capped {
            return self.fail(XrayError::Protocol(format!(
                "USB shutdown delay readback {readback} differs from requested {delay}"
            )));
        }
        self.health.usb_shutdown_delay = Some(readback);
        Ok(self.health())
    }

    pub fn disconnect(&mut self) -> Result<(), XrayError> {
        let off_result = if self.transport.is_some() {
            self.force_off_inner()
        } else {
            Ok(())
        };
        self.transport = None;
        self.health.connected = false;
        self.requested_setpoint = None;
        if let Err(error) = off_result {
            self.health.beam_off_confirmed = false;
            self.health.last_error = Some(error.to_string());
            return Err(error);
        }
        Ok(())
    }

    fn connect_transport(
        &mut self,
        port: String,
        serial: String,
        transport: Box<dyn BinaryTransport>,
    ) -> Result<(), XrayError> {
        self.transport = Some(transport);
        self.health = XrayHealth {
            connected: true,
            port: Some(port),
            serial: Some(serial),
            ..XrayHealth::default()
        };
        let detected = match self.detect_beam_state() {
            Ok(emitting) => emitting,
            Err(error) => {
                self.transport = None;
                self.health.connected = false;
                self.health.last_error = Some(error.to_string());
                return Err(error);
            }
        };
        if !detected {
            // Beam already OFF at connect: send the explicit OFF command so the
            // session starts from a confirmed-safe baseline.
            if let Err(error) = self.force_off_inner() {
                self.transport = None;
                self.health.connected = false;
                self.health.last_error = Some(error.to_string());
                return Err(error);
            }
        }
        Ok(())
    }

    /// Reads the current hardware beam state without commanding anything.
    /// Returns Ok(true) when the source is emitting right now (for example
    /// after a killed process left the tube on): the connection is accepted,
    /// `beam_on` is reported and the operator must explicitly turn it off.
    fn detect_beam_state(&mut self) -> Result<bool, XrayError> {
        let setpoints = self.read_setpoints()?;
        let status = self.read_status()?;
        let emitting = setpoints.enabled || status.kv_enabled || status.ua_enabled;
        if emitting {
            self.health.connected = true;
            self.health.beam_on = true;
            self.health.beam_off_confirmed = false;
            self.health.locked = Some(status.locked);
            self.health.set_voltage_kv = Some(setpoints.voltage_kv);
            self.health.set_current_ua = Some(setpoints.current_ua);
            self.health.voltage_kv = Some(status.voltage_kv);
            self.health.current_ua = Some(status.current_ua);
            self.health.temperature_c = Some(status.temperature_c);
            self.health.last_error = None;
        }
        Ok(emitting)
    }

    fn force_off_inner(&mut self) -> Result<(), XrayError> {
        self.transact(CMD_SET_XRAY_ENABLE, &[0], 4)?;
        let setpoints = self.read_setpoints()?;
        let status = self.read_status()?;
        self.require_off(&setpoints, &status)?;
        self.apply_health(&setpoints, &status);
        self.last_off = Some(Instant::now());
        Ok(())
    }

    fn beam_on_inner(&mut self, cancel: &AtomicBool) -> Result<(), XrayError> {
        let (requested_voltage, requested_current) = self.requested_setpoint.ok_or_else(|| {
            XrayError::Safety("beam enable requires a setpoint verified in this session".into())
        })?;
        validate_setpoint(requested_voltage, requested_current)?;
        let setpoints = self.read_setpoints()?;
        if !within_setpoint_tolerance(&setpoints, requested_voltage, requested_current) {
            return Err(XrayError::Safety(
                "Moxtek setpoint readback no longer matches the requested operating point".into(),
            ));
        }
        let initial = self.read_status()?;
        if initial.locked {
            return Err(XrayError::Safety("Moxtek digital interlock is open".into()));
        }
        if !initial.temperature_c.is_finite() || initial.temperature_c > MAX_CASE_TEMPERATURE_C {
            return Err(XrayError::Safety(format!(
                "Moxtek case temperature {:.2} C exceeds {:.0} C",
                initial.temperature_c, MAX_CASE_TEMPERATURE_C
            )));
        }

        while self
            .last_off
            .is_some_and(|off| off.elapsed() < MIN_REENABLE_INTERVAL)
        {
            if cancel.load(Ordering::SeqCst) {
                return Err(XrayError::Safety("beam enable cancelled".into()));
            }
            thread::sleep(Duration::from_millis(25));
        }
        if cancel.load(Ordering::SeqCst) {
            return Err(XrayError::Safety("beam enable cancelled".into()));
        }
        self.transact(CMD_SET_XRAY_ENABLE, &[1], 4)?;

        let deadline = Instant::now() + EMISSION_STABILITY_TIMEOUT;
        let mut stable_samples = 0_u8;
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(XrayError::Safety("beam enable cancelled".into()));
            }
            if Instant::now() >= deadline {
                return Err(XrayError::Safety(
                    "Moxtek measured output did not stabilize within 8 seconds".into(),
                ));
            }
            let status = self.read_status()?;
            let setpoints = self.read_setpoints()?;
            if status.locked {
                return Err(XrayError::Safety(
                    "Moxtek digital interlock opened during enable".into(),
                ));
            }
            if !status.temperature_c.is_finite() || status.temperature_c > MAX_CASE_TEMPERATURE_C {
                return Err(XrayError::Safety(format!(
                    "Moxtek case temperature {:.2} C exceeds {:.0} C",
                    status.temperature_c, MAX_CASE_TEMPERATURE_C
                )));
            }
            let stable = setpoints.enabled
                && status.kv_enabled
                && status.ua_enabled
                && (status.voltage_kv - requested_voltage).abs()
                    <= VOLTAGE_READBACK_TOLERANCE_KV
                && (status.current_ua - requested_current).abs()
                    <= CURRENT_READBACK_TOLERANCE_UA;
            stable_samples = if stable { stable_samples + 1 } else { 0 };
            if stable_samples >= 2 {
                self.health.connected = true;
                self.health.beam_off_confirmed = false;
                self.health.beam_on = true;
                self.health.locked = Some(false);
                self.health.set_voltage_kv = Some(setpoints.voltage_kv);
                self.health.set_current_ua = Some(setpoints.current_ua);
                self.health.voltage_kv = Some(status.voltage_kv);
                self.health.current_ua = Some(status.current_ua);
                self.health.temperature_c = Some(status.temperature_c);
                self.health.last_error = None;
                return Ok(());
            }
            thread::sleep(EMISSION_POLL_INTERVAL);
        }
    }

    fn require_off(&mut self, setpoints: &Setpoints, status: &Status) -> Result<(), XrayError> {
        if setpoints.enabled || status.kv_enabled || status.ua_enabled {
            return self.fail(XrayError::Safety(
                "Moxtek did not confirm command and measured output flags OFF".into(),
            ));
        }
        Ok(())
    }

    fn apply_health(&mut self, setpoints: &Setpoints, status: &Status) {
        self.health.connected = true;
        self.health.beam_off_confirmed = true;
        self.health.beam_on = false;
        self.health.locked = Some(status.locked);
        self.health.set_voltage_kv = Some(setpoints.voltage_kv);
        self.health.set_current_ua = Some(setpoints.current_ua);
        self.health.voltage_kv = Some(status.voltage_kv);
        self.health.current_ua = Some(status.current_ua);
        self.health.temperature_c = Some(status.temperature_c);
        self.health.last_error = None;
    }

    fn read_setpoints(&mut self) -> Result<Setpoints, XrayError> {
        let response = self.transact(CMD_GET_SETPOINTS, &[], 8)?;
        let enabled = match response[7] {
            0 => false,
            1 => true,
            value => {
                return Err(XrayError::Protocol(format!(
                    "invalid beam state byte {value}"
                )))
            }
        };
        Ok(Setpoints {
            voltage_kv: raw_to_kv(read_u16(&response, 3)?),
            current_ua: raw_to_ua(read_u16(&response, 5)?),
            enabled,
        })
    }

    fn read_status(&mut self) -> Result<Status, XrayError> {
        let response = self.transact(CMD_GET_STATUS, &[], 19)?;
        Ok(Status {
            voltage_kv: raw_to_kv(read_u16(&response, 3)?),
            current_ua: raw_to_ua(read_u16(&response, 5)?),
            temperature_c: raw_to_temp(read_u16(&response, 8)?),
            locked: response[7] > 0,
            kv_enabled: response[10] != 0,
            ua_enabled: response[11] != 0,
        })
    }

    fn write_setpoint(&mut self, command: u8, raw: u16) -> Result<(), XrayError> {
        self.transact(command, &raw.to_le_bytes(), 5)?;
        Ok(())
    }

    fn verify_one_setpoint(
        &mut self,
        voltage_kv: Option<f64>,
        current_ua: Option<f64>,
    ) -> Result<(), XrayError> {
        let actual = self.read_setpoints()?;
        if actual.enabled {
            return Err(XrayError::Safety(
                "setpoint write unexpectedly enabled the beam command".into(),
            ));
        }
        if voltage_kv.is_some_and(|value| {
            (actual.voltage_kv - value).abs() > VOLTAGE_READBACK_TOLERANCE_KV
        }) || current_ua.is_some_and(|value| {
            (actual.current_ua - value).abs() > CURRENT_READBACK_TOLERANCE_UA
        }) {
            return Err(XrayError::Protocol(
                "Moxtek did not read back the requested setpoint".into(),
            ));
        }
        Ok(())
    }

    fn fail<T>(&mut self, error: XrayError) -> Result<T, XrayError> {
        self.health.beam_off_confirmed = false;
        self.health.last_error = Some(error.to_string());
        Err(error)
    }

    fn transact(
        &mut self,
        command: u8,
        payload: &[u8],
        expected_length: usize,
    ) -> Result<Vec<u8>, XrayError> {
        let transport = self
            .transport
            .as_mut()
            .ok_or_else(|| XrayError::Transport("Moxtek adapter is not connected".into()))?;
        let payload_length = u8::try_from(payload.len())
            .map_err(|_| XrayError::Protocol("payload is too long".into()))?;
        let mut packet = vec![PACKET_HEADER, command, payload_length];
        packet.extend_from_slice(payload);
        transport
            .clear_input()
            .map_err(|error| XrayError::Transport(format!("input purge failed: {error}")))?;
        transport
            .write_all_bytes(&packet)
            .map_err(|error| XrayError::Transport(format!("write failed: {error}")))?;
        let mut response = vec![0; expected_length];
        transport
            .read_exact_bytes(&mut response)
            .map_err(|error| XrayError::Transport(format!("read failed: {error}")))?;
        if response.len() < 3
            || response[0] != PACKET_HEADER
            || response[1] != command
            || usize::from(response[2]) != expected_length.saturating_sub(3)
        {
            return Err(XrayError::Protocol(format!(
                "invalid response for command 0x{command:02X}"
            )));
        }
        Ok(response)
    }
}

impl Drop for MoxtekAdapter {
    fn drop(&mut self) {
        let _ = self.disconnect();
    }
}

fn validate_setpoint(voltage_kv: f64, current_ua: f64) -> Result<(), XrayError> {
    if !voltage_kv.is_finite()
        || !current_ua.is_finite()
        || !(MIN_VOLTAGE_KV..=MAX_VOLTAGE_KV).contains(&voltage_kv)
        || !(0.0..=MAX_CURRENT_UA).contains(&current_ua)
    {
        return Err(XrayError::Parameter(format!(
            "requested setpoint must be {MIN_VOLTAGE_KV:.0}..{MAX_VOLTAGE_KV:.0} kV and 0..{MAX_CURRENT_UA:.0} uA"
        )));
    }
    let power_w = voltage_kv * current_ua / 1_000.0;
    if power_w > MAX_SETPOINT_POWER_W + 1e-12 {
        return Err(XrayError::Parameter(format!(
            "requested setpoint exceeds {MAX_SETPOINT_POWER_W:.0} W: {voltage_kv:.4} kV * {current_ua:.4} uA = {power_w:.4} W"
        )));
    }
    Ok(())
}

fn within_setpoint_tolerance(setpoints: &Setpoints, voltage_kv: f64, current_ua: f64) -> bool {
    !setpoints.enabled
        && (setpoints.voltage_kv - voltage_kv).abs() <= VOLTAGE_READBACK_TOLERANCE_KV
        && (setpoints.current_ua - current_ua).abs() <= CURRENT_READBACK_TOLERANCE_UA
}

fn is_exact_moxtek(port: &SerialPortInfo) -> bool {
    matches!(
        &port.port_type,
        SerialPortType::UsbPort(usb)
            if usb.vid == MOXTEK_VID
                && usb.pid == MOXTEK_PID
                && usb.serial_number.as_deref().is_some_and(is_moxtek_serial)
    )
}

fn is_moxtek_serial(value: &str) -> bool {
    if value == MOXTEK_SERIAL {
        return true;
    }
    // FTDI's Windows VCP child appends the interface letter to the USB
    // descriptor serial (FTDIBUS ... +168249A). Accept exactly that one
    // documented interface suffix; other serials remain rejected.
    cfg!(windows) && value == format!("{MOXTEK_SERIAL}A")
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, XrayError> {
    let pair = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| XrayError::Protocol("short numeric field".into()))?;
    Ok(u16::from_le_bytes([pair[0], pair[1]]))
}

fn kv_to_raw(value: f64) -> Result<u16, XrayError> {
    scaled_to_raw(value, VOLTAGE_SCALE, "voltage")
}

fn ua_to_raw(value: f64) -> Result<u16, XrayError> {
    scaled_to_raw(value, CURRENT_SCALE, "current")
}

fn scaled_to_raw(value: f64, scale: f64, name: &str) -> Result<u16, XrayError> {
    let raw = (value / scale * DAC_SCALE).round();
    if !raw.is_finite() || !(0.0..=f64::from(u16::MAX)).contains(&raw) {
        return Err(XrayError::Parameter(format!("{name} cannot be encoded")));
    }
    Ok(raw as u16)
}

fn raw_to_kv(raw: u16) -> f64 {
    f64::from(raw) / DAC_SCALE * VOLTAGE_SCALE
}

fn raw_to_ua(raw: u16) -> f64 {
    f64::from(raw) / DAC_SCALE * CURRENT_SCALE
}

fn raw_to_temp(raw: u16) -> f64 {
    (f64::from(raw) - TEMP_OFFSET) / TEMP_SCALE
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct FakeState {
        writes: Vec<Vec<u8>>,
        response: Option<Vec<u8>>,
        set_voltage_raw: u16,
        set_current_raw: u16,
        enabled: bool,
        measured_voltage_raw: u16,
        measured_current_raw: u16,
        temperature_raw: u16,
        locked: bool,
        kv_enabled: bool,
        ua_enabled: bool,
        usb_auto_shutdown: bool,
    }

    impl Default for FakeState {
        fn default() -> Self {
            Self {
                writes: Vec::new(),
                response: None,
                set_voltage_raw: 0,
                set_current_raw: 0,
                enabled: false,
                measured_voltage_raw: 0,
                measured_current_raw: 0,
                temperature_raw: 175,
                locked: false,
                kv_enabled: false,
                ua_enabled: false,
                usb_auto_shutdown: true,
            }
        }
    }

    struct FakeTransport {
        state: Arc<Mutex<FakeState>>,
    }

    impl FakeTransport {
        fn lock(&self) -> std::sync::MutexGuard<'_, FakeState> {
            self.state.lock().unwrap_or_else(|poison| poison.into_inner())
        }
    }

    impl BinaryTransport for FakeTransport {
        fn clear_input(&mut self) -> io::Result<()> {
            self.lock().response = None;
            Ok(())
        }

        fn write_all_bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
            let mut state = self.lock();
            state.writes.push(bytes.to_vec());
            let command = bytes[1];
            let response = match command {
                CMD_SET_VOLTAGE => {
                    state.set_voltage_raw = u16::from_le_bytes([bytes[3], bytes[4]]);
                    vec![PACKET_HEADER, command, 2, bytes[3], bytes[4]]
                }
                CMD_SET_CURRENT => {
                    state.set_current_raw = u16::from_le_bytes([bytes[3], bytes[4]]);
                    vec![PACKET_HEADER, command, 2, bytes[3], bytes[4]]
                }
                CMD_SET_XRAY_ENABLE => {
                    state.enabled = bytes[3] != 0;
                    if state.enabled {
                        state.kv_enabled = true;
                        state.ua_enabled = true;
                        state.measured_voltage_raw = state.set_voltage_raw;
                        state.measured_current_raw = state.set_current_raw;
                    } else {
                        state.kv_enabled = false;
                        state.ua_enabled = false;
                        state.measured_voltage_raw = 0;
                        state.measured_current_raw = 0;
                    }
                    vec![PACKET_HEADER, command, 1, bytes[3]]
                }
                CMD_USB_AUTO_SHUTDOWN => {
                    state.usb_auto_shutdown = bytes[3] != 0;
                    vec![PACKET_HEADER, command, 1, bytes[3]]
                }
                CMD_GET_USB_SHUTDOWN_TIMER => {
                    vec![PACKET_HEADER, command, 2, 5, 0]
                }
                CMD_GET_SETPOINTS => {
                    let voltage = state.set_voltage_raw.to_le_bytes();
                    let current = state.set_current_raw.to_le_bytes();
                    vec![
                        PACKET_HEADER,
                        command,
                        5,
                        voltage[0],
                        voltage[1],
                        current[0],
                        current[1],
                        u8::from(state.enabled),
                    ]
                }
                CMD_GET_STATUS => {
                    let voltage = state.measured_voltage_raw.to_le_bytes();
                    let current = state.measured_current_raw.to_le_bytes();
                    let temperature = state.temperature_raw.to_le_bytes();
                    vec![
                        PACKET_HEADER,
                        command,
                        16,
                        voltage[0],
                        voltage[1],
                        current[0],
                        current[1],
                        u8::from(state.locked),
                        temperature[0],
                        temperature[1],
                        u8::from(state.kv_enabled),
                        u8::from(state.ua_enabled),
                        0,
                        0,
                        0,
                        0,
                        0,
                        0,
                        0,
                    ]
                }
                _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "unknown command")),
            };
            state.response = Some(response);
            Ok(())
        }

        fn read_exact_bytes(&mut self, bytes: &mut [u8]) -> io::Result<()> {
            let response = self
                .lock()
                .response
                .take()
                .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "no response"))?;
            if response.len() != bytes.len() {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "wrong response length"));
            }
            bytes.copy_from_slice(&response);
            Ok(())
        }
    }

    fn connected_adapter(state: Arc<Mutex<FakeState>>) -> MoxtekAdapter {
        let mut adapter = MoxtekAdapter::new();
        adapter
            .connect_transport(
                "COM-XRAY".into(),
                MOXTEK_SERIAL.into(),
                Box::new(FakeTransport { state }),
            )
            .unwrap();
        adapter
    }

    #[test]
    fn connect_reads_state_then_confirms_off_before_accepting_device() {
        let state = Arc::new(Mutex::new(FakeState::default()));
        let adapter = connected_adapter(state.clone());
        let health = adapter.health();
        assert!(health.connected && health.beam_off_confirmed && !health.beam_on);
        let writes = state.lock().unwrap().writes.clone();
        // The connect handshake first reads the live state, then sends the
        // explicit OFF command as the safe baseline.
        assert_eq!(writes[0], vec![PACKET_HEADER, CMD_GET_SETPOINTS, 0]);
        assert_eq!(writes[1], vec![PACKET_HEADER, CMD_GET_STATUS, 0]);
        assert!(writes
            .iter()
            .any(|packet| packet.as_slice() == [PACKET_HEADER, CMD_SET_XRAY_ENABLE, 1, 0]));
    }

    #[test]
    fn connect_reports_a_beam_left_on_without_commanding_off() {
        // Simulates a previous process killed mid-beam: the tube is still
        // emitting when the new session connects.
        let state = Arc::new(Mutex::new(FakeState {
            set_voltage_raw: kv_to_raw(60.0).unwrap(),
            set_current_raw: ua_to_raw(200.0).unwrap(),
            enabled: true,
            measured_voltage_raw: kv_to_raw(60.0).unwrap(),
            measured_current_raw: ua_to_raw(200.0).unwrap(),
            kv_enabled: true,
            ua_enabled: true,
            ..FakeState::default()
        }));
        let mut adapter = connected_adapter(state.clone());
        let health = adapter.health();
        assert!(health.connected && health.beam_on && !health.beam_off_confirmed);
        assert!(!state
            .lock()
            .unwrap()
            .writes
            .iter()
            .any(|packet| packet.as_slice() == [PACKET_HEADER, CMD_SET_XRAY_ENABLE, 1, 0]));
        // The operator can still turn it off explicitly.
        adapter.force_off().unwrap();
        assert!(adapter.health().beam_off_confirmed && !adapter.health().beam_on);
    }

    #[test]
    fn measured_output_flag_keeps_connection_fail_closed() {
        let state = Arc::new(Mutex::new(FakeState::default()));
        let mut adapter = connected_adapter(state.clone());
        state.lock().unwrap().kv_enabled = true;
        let result = adapter.refresh_off_status();
        assert!(matches!(result, Err(XrayError::Safety(_))));
        assert!(!adapter.health().beam_off_confirmed);
    }

    #[test]
    fn sixty_kv_two_hundred_ua_are_encoded_exactly_and_verified() {
        let state = Arc::new(Mutex::new(FakeState {
            set_voltage_raw: kv_to_raw(60.0).unwrap(),
            set_current_raw: ua_to_raw(100.0).unwrap(),
            ..FakeState::default()
        }));
        let mut adapter = connected_adapter(state.clone());
        let health = adapter.set_parameters(60.0, 200.0).unwrap();
        assert!((health.set_voltage_kv.unwrap() - 60.0).abs() < 0.1);
        assert!((health.set_current_ua.unwrap() - 200.0).abs() < 1.0);
        let writes = &state.lock().unwrap().writes;
        assert!(writes.contains(&vec![PACKET_HEADER, CMD_SET_CURRENT, 2, 0, 0]));
        assert!(writes.contains(&{
            let [low, high] = kv_to_raw(60.0).unwrap().to_le_bytes();
            vec![PACKET_HEADER, CMD_SET_VOLTAGE, 2, low, high]
        }));
        assert!(writes.contains(&{
            let [low, high] = ua_to_raw(200.0).unwrap().to_le_bytes();
            vec![PACKET_HEADER, CMD_SET_CURRENT, 2, low, high]
        }));
    }

    #[test]
    fn requested_values_enforce_hard_limits_but_monitor_bias_does_not_block_off() {
        assert!(validate_setpoint(60.0, 200.0).is_ok());
        assert!(validate_setpoint(60.0, 200.1).is_err());
        assert!(validate_setpoint(70.1, 100.0).is_err());
        assert!(validate_setpoint(70.0, 1_000.1).is_err());

        let state = Arc::new(Mutex::new(FakeState::default()));
        let mut adapter = connected_adapter(state.clone());
        {
            let mut fake = state.lock().unwrap();
            fake.measured_voltage_raw = kv_to_raw(70.05).unwrap();
            fake.measured_current_raw = ua_to_raw(1_000.05).unwrap();
        }
        adapter.refresh_off_status().unwrap();
        let health = adapter.health();
        assert!(health.beam_off_confirmed);
        assert!(health.voltage_kv.unwrap() > MAX_VOLTAGE_KV);
        assert!(health.current_ua.unwrap() > MAX_CURRENT_UA);
    }

    #[test]
    fn windows_ftdi_interface_suffix_preserves_exact_device_identity() {
        assert!(is_moxtek_serial(MOXTEK_SERIAL));
        if cfg!(windows) {
            assert!(is_moxtek_serial("168249A"));
        }
        assert!(!is_moxtek_serial("168249B"));
        assert!(!is_moxtek_serial("1682490"));
        assert!(!is_moxtek_serial("OTHER"));
    }

    #[test]
    fn disconnect_repeats_off_confirmation() {
        let state = Arc::new(Mutex::new(FakeState::default()));
        let mut adapter = connected_adapter(state.clone());
        adapter.disconnect().unwrap();
        let off_writes = state
            .lock()
            .unwrap()
            .writes
            .iter()
            .filter(|packet| packet.as_slice() == [PACKET_HEADER, CMD_SET_XRAY_ENABLE, 1, 0])
            .count();
        assert_eq!(off_writes, 2);
    }

    #[test]
    fn usb_auto_shutdown_command_is_written_and_timer_read_back() {
        let state = Arc::new(Mutex::new(FakeState::default()));
        let mut adapter = connected_adapter(state.clone());
        adapter.set_usb_auto_shutdown(false).unwrap();
        assert!(!state.lock().unwrap().usb_auto_shutdown);
        assert_eq!(adapter.health().usb_auto_shutdown, Some(false));
        assert!(state
            .lock()
            .unwrap()
            .writes
            .contains(&vec![PACKET_HEADER, CMD_USB_AUTO_SHUTDOWN, 1, 0]));
        adapter.set_usb_auto_shutdown(true).unwrap();
        assert!(state.lock().unwrap().usb_auto_shutdown);
        let delay = adapter.read_usb_shutdown_timer().unwrap();
        assert_eq!(delay, 5);
        assert_eq!(adapter.health().usb_shutdown_delay, Some(5));
    }

    #[test]
    fn beam_enable_requires_verified_parameters_and_converged_readback() {
        let state = Arc::new(Mutex::new(FakeState::default()));
        let mut adapter = connected_adapter(state.clone());
        adapter.set_parameters(60.0, 200.0).unwrap();
        adapter.last_off = Some(Instant::now() - MIN_REENABLE_INTERVAL);
        let cancel = AtomicBool::new(false);
        let health = adapter.beam_on(&cancel).unwrap();
        assert!(!health.beam_off_confirmed);
        assert!(state.lock().unwrap().enabled);
        adapter.force_off().unwrap();
        assert!(adapter.health().beam_off_confirmed);
    }
}
