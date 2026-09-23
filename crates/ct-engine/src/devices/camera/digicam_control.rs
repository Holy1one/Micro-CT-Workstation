//! Nikon camera driver for the current PC-direct control topology.
//!
//! This module wraps the external DigiCamControl command-line program. It is
//! intentionally responsible only for camera discovery, configuration,
//! capture, and host-side file confirmation. Scan ordering and X-ray safety
//! belong to the application layer, not to this device driver.

use serde::Serialize;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(60);
const POLL_INTERVAL: Duration = Duration::from_millis(100);
// D7100 timed-shutter range; Bulb/Time require a different capture protocol.
pub const EXPOSURE_MIN_MS: f64 = 0.125;
pub const EXPOSURE_MAX_MS: f64 = 30_000.0;

pub fn valid_exposure_ms(value: f64) -> bool {
    value.is_finite() && (EXPOSURE_MIN_MS..=EXPOSURE_MAX_MS).contains(&value)
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraHealth {
    pub connected: bool,
    pub executable: Option<String>,
    pub serial: Option<String>,
    pub transfer_policy: Option<String>,
    pub last_capture: Option<String>,
    pub last_error: Option<String>,
    pub exposure_min_ms: Option<f64>,
    pub exposure_max_ms: Option<f64>,
}

#[derive(Debug)]
pub enum CameraError {
    Cancelled,
    Unavailable(String),
    Command(String),
    Safety(String),
    Io(String),
    Timeout(String),
}

impl std::fmt::Display for CameraError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => write!(formatter, "camera capture cancelled"),
            Self::Unavailable(message) => write!(formatter, "camera unavailable: {message}"),
            Self::Command(message) => write!(formatter, "camera command failed: {message}"),
            Self::Safety(message) => write!(formatter, "camera safety check failed: {message}"),
            Self::Io(message) => write!(formatter, "camera I/O failed: {message}"),
            Self::Timeout(message) => write!(formatter, "camera timeout: {message}"),
        }
    }
}

impl std::error::Error for CameraError {}

struct CommandOutput {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

pub struct DigiCamControlAdapter {
    executable: PathBuf,
    health: CameraHealth,
}

impl DigiCamControlAdapter {
    pub fn discover() -> Result<Self, CameraError> {
        let executable = candidate_paths()
            .into_iter()
            .find(|path| path.is_file())
            .ok_or_else(|| {
                CameraError::Unavailable(
                    "CameraControlRemoteCmd.exe was not found; set DIGICAMCONTROL_REMOTE_EXE"
                        .into(),
                )
            })?;
        let health = CameraHealth {
            executable: Some(executable.display().to_string()),
            ..CameraHealth::default()
        };
        Ok(Self { executable, health })
    }

    pub fn health(&self) -> CameraHealth {
        self.health.clone()
    }

    pub fn set_exposure_ms(&mut self, exposure_ms: f64) -> Result<String, CameraError> {
        if !valid_exposure_ms(exposure_ms) {
            return Err(CameraError::Safety(
                "D7100 timed exposure must be within 0.125..30000 ms".into(),
            ));
        }
        if !self.health.connected {
            self.connect()?;
        }
        let supported = self.remote("list shutterspeed", COMMAND_TIMEOUT)?;
        let shutter = select_shutter(&supported.stdout, exposure_ms)?;
        self.remote(
            &format!("set shutterspeed {shutter}"),
            COMMAND_TIMEOUT,
        )?;
        let readback = self.remote("get shutterspeed", COMMAND_TIMEOUT)?;
        let actual = useful_lines(&readback.stdout).into_iter().last()
            .and_then(|value| shutter_ms(&value));
        if !actual.is_some_and(|value| (value - exposure_ms).abs() <= exposure_ms.max(1.0) * 1e-6) {
            return Err(CameraError::Safety(format!(
                "camera did not confirm requested exposure {exposure_ms} ms; readback={:?}", readback.stdout.trim()
            )));
        }
        Ok(shutter)
    }

    pub fn connect(&mut self) -> Result<CameraHealth, CameraError> {
        let result = self.remote("list cameras", COMMAND_TIMEOUT)?;
        let serial = useful_lines(&result.stdout)
            .into_iter()
            .last()
            .ok_or_else(|| CameraError::Unavailable("digiCamControl reported no camera".into()))?;
        self.remote("set transfer Save_to_PC_only", COMMAND_TIMEOUT)?;
        let transfer = self.remote("get transfer", COMMAND_TIMEOUT)?;
        let transfer_policy = useful_lines(&transfer.stdout).into_iter().last().unwrap_or_default();
        if normalize(&transfer_policy) != "savetopconly" {
            return self.fail(CameraError::Safety(format!(
                "expected Save to PC only, got {transfer_policy:?}"
            )));
        }
        let supported = self.remote("list shutterspeed", COMMAND_TIMEOUT)?;
        let values: Vec<f64> = useful_lines(&supported.stdout).iter()
            .filter_map(|value| shutter_ms(value)).filter(|value| valid_exposure_ms(*value)).collect();
        if values.is_empty() {
            return self.fail(CameraError::Safety("camera returned no supported timed exposures".into()));
        }
        self.health.exposure_min_ms = values.iter().copied().reduce(f64::min);
        self.health.exposure_max_ms = values.iter().copied().reduce(f64::max);
        self.health.connected = true;
        self.health.serial = Some(serial);
        self.health.transfer_policy = Some("Save to PC only".into());
        self.health.last_error = None;
        Ok(self.health())
    }

    pub fn capture_test(
        &mut self,
        save_path: &Path,
        task_id: &str,
    ) -> Result<PathBuf, CameraError> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        self.capture_named(save_path, task_id, &format!("camera-test-{stamp}.nef"), None)
    }

    pub fn capture_frame(
        &mut self,
        save_path: &Path,
        task_id: &str,
        index: u32,
    ) -> Result<PathBuf, CameraError> {
        self.capture_named(save_path, task_id, &format!("frame-{index:04}.nef"), None)
    }

    pub fn capture_frame_cancellable(&mut self, save_path: &Path, task_id: &str, index: u32, cancel: &AtomicBool) -> Result<PathBuf, CameraError> {
        self.capture_named(save_path, task_id, &format!("frame-{index:04}.nef"), Some(cancel))
    }

    /// Path whose appearance means the shutter exposure has completed and
    /// DigiCamControl has started transferring the NEF to the host.
    pub fn frame_transfer_started(save_path: &Path, task_id: &str, index: u32) -> bool {
        let frames = save_path.join(task_id).join("frames");
        let filename = format!("frame-{index:04}.nef");
        capture_staging_candidates(&frames, &filename)
            .iter()
            .any(|path| path.exists())
    }

    fn capture_named(
        &mut self,
        save_path: &Path,
        task_id: &str,
        filename: &str,
        cancel: Option<&AtomicBool>,
    ) -> Result<PathBuf, CameraError> {
        check_capture_cancel(cancel)?;
        if !self.health.connected {
            self.connect()?;
        }
        validate_task_id(task_id)?;
        let transfer = self.remote_cancellable("get transfer", COMMAND_TIMEOUT, cancel)?;
        let policy = useful_lines(&transfer.stdout).into_iter().last().unwrap_or_default();
        if normalize(&policy) != "savetopconly" {
            return self.fail(CameraError::Safety(
                "transfer policy changed; capture refused".into(),
            ));
        }

        let frames = save_path.join(task_id).join("frames");
        fs::create_dir_all(&frames).map_err(|error| CameraError::Io(error.to_string()))?;
        let destination = frames.join(filename);
        let (capture_target, appended_staging) = capture_staging_paths(&frames, filename);
        let staging_candidates = capture_staging_candidates(&frames, filename);
        if destination.exists()
            || capture_target.exists()
            || staging_candidates.iter().any(|path| path.exists())
        {
            return Err(CameraError::Safety(
                "generated capture destination already exists".into(),
            ));
        }

        // DigiCamControl versions either append ".nef" to the hidden target or
        // replace its ".part" suffix. Accept both observed vendor behaviors,
        // then atomically commit the stable file to the public frame name.
        let command = format!("capture {}", capture_target.display());
        check_capture_cancel(cancel)?;
        let capture_output = match self.remote_cancellable(&command, CAPTURE_TIMEOUT, cancel) {
            Ok(output) => output,
            Err(error) => {
                let _ = fs::remove_file(&capture_target);
                for staging in &staging_candidates {
                    let _ = fs::remove_file(staging);
                }
                return self.fail(error);
            }
        };
        let staging = match wait_for_stable_file(&staging_candidates, CAPTURE_TIMEOUT, cancel) {
            Ok(path) => path,
            Err(error) => {
                let _ = fs::remove_file(&capture_target);
                for staging in &staging_candidates {
                    let _ = fs::remove_file(staging);
                }
                return self.fail(match error {
                    CameraError::Cancelled => CameraError::Cancelled,
                    other => CameraError::Timeout(format!(
                        "{other}; RemoteCmd stdout={:?}; stderr={:?}",
                        capture_output.stdout.trim(),
                        capture_output.stderr.trim()
                    )),
                });
            }
        };
        if let Err(error) = commit_when_released(&staging, &destination, CAPTURE_TIMEOUT, cancel) {
            let _ = fs::remove_file(&capture_target);
            for staging in &staging_candidates {
                let _ = fs::remove_file(staging);
            }
            return self.fail(error);
        }
        if staging != appended_staging {
            let _ = fs::remove_file(&appended_staging);
        }
        self.health.last_capture = Some(destination.display().to_string());
        self.health.last_error = None;
        Ok(destination)
    }

    pub fn disconnect(&mut self) {
        self.health.connected = false;
        self.health.serial = None;
        self.health.transfer_policy = None;
    }

    fn fail<T>(&mut self, error: CameraError) -> Result<T, CameraError> {
        self.health.connected = false;
        self.health.last_error = Some(error.to_string());
        Err(error)
    }

    fn remote(&self, command_text: &str, timeout: Duration) -> Result<CommandOutput, CameraError> {
        self.remote_cancellable(command_text, timeout, None)
    }

    fn remote_cancellable(&self, command_text: &str, timeout: Duration, cancel: Option<&AtomicBool>) -> Result<CommandOutput, CameraError> {
        let output = run_with_timeout(
            &self.executable,
            [
                OsString::from("/c"),
                OsString::from(command_text),
                OsString::from("/clean"),
            ],
            timeout,
            cancel,
        )?;
        if !output.status.success() {
            return Err(CameraError::Command(format!(
                "exit {:?}; stderr={}",
                output.status.code(),
                output.stderr.trim()
            )));
        }
        let combined = format!("{}\n{}", output.stdout, output.stderr).to_ascii_lowercase();
        if combined.contains("error") || combined.contains("exception") {
            return Err(CameraError::Command(combined.trim().to_owned()));
        }
        Ok(output)
    }
}

fn candidate_paths() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for key in [
        "DIGICAMCONTROL_REMOTE_EXE",
        "DIGICAMCONTROL_REMOTE_CMD",
        "CAMERACONTROL_REMOTE_CMD",
    ] {
        if let Some(value) = env::var_os(key) {
            candidates.push(PathBuf::from(value));
        }
    }
    candidates.push(PathBuf::from(
        r"E:\Application\digiCamControl\CameraControlRemoteCmd.exe",
    ));
    for key in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
        if let Some(root) = env::var_os(key) {
            candidates.push(
                PathBuf::from(root)
                    .join("digiCamControl")
                    .join("CameraControlRemoteCmd.exe"),
            );
        }
    }
    candidates.sort();
    candidates.dedup();
    candidates
}

fn run_with_timeout<I>(
    executable: &Path,
    args: I,
    timeout: Duration,
    cancel: Option<&AtomicBool>,
) -> Result<CommandOutput, CameraError>
where
    I: IntoIterator<Item = OsString>,
{
    check_capture_cancel(cancel)?;
    let mut command = Command::new(executable);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| CameraError::Unavailable(error.to_string()))?;
    let deadline = Instant::now() + timeout;
    loop {
        if check_capture_cancel(cancel).is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(CameraError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child
                    .wait_with_output()
                    .map_err(|error| CameraError::Io(error.to_string()))?;
                return Ok(CommandOutput {
                    status: output.status,
                    stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                });
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL_INTERVAL),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(CameraError::Timeout(format!(
                    "{} exceeded {} seconds",
                    executable.display(),
                    timeout.as_secs()
                )));
            }
            Err(error) => return Err(CameraError::Io(error.to_string())),
        }
    }
}

fn useful_lines(value: &str) -> Vec<String> {
    value
        .lines()
        .map(|line| line.trim().trim_matches('"'))
        .filter(|line| {
            !line.is_empty()
                && !line.starts_with("digiCamControl remote command line utility")
        })
        .map(str::to_owned)
        .collect()
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn validate_task_id(value: &str) -> Result<(), CameraError> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed.contains(['/', '\\', '\0'])
    {
        return Err(CameraError::Safety("task ID is not a safe path component".into()));
    }
    Ok(())
}

fn capture_staging_paths(frames: &Path, filename: &str) -> (PathBuf, PathBuf) {
    let stem = filename.strip_suffix(".nef").unwrap_or(filename);
    let capture_target = frames.join(format!(".{stem}.part"));
    let staging = frames.join(format!(".{stem}.part.nef"));
    (capture_target, staging)
}

fn capture_staging_candidates(frames: &Path, filename: &str) -> Vec<PathBuf> {
    let stem = filename.strip_suffix(".nef").unwrap_or(filename);
    let (_, appended) = capture_staging_paths(frames, filename);
    vec![appended, frames.join(format!(".{stem}.nef"))]
}

fn check_capture_cancel(cancel: Option<&AtomicBool>) -> Result<(), CameraError> {
    if cancel.is_some_and(|flag| flag.load(Ordering::SeqCst)) { Err(CameraError::Cancelled) } else { Ok(()) }
}

fn wait_for_stable_file(paths: &[PathBuf], timeout: Duration, cancel: Option<&AtomicBool>) -> Result<PathBuf, CameraError> {
    let deadline = Instant::now() + timeout;
    let mut previous: Option<(PathBuf, u64)> = None;
    while Instant::now() < deadline {
        check_capture_cancel(cancel)?;
        let mut found = false;
        for path in paths {
            match fs::metadata(path) {
                Ok(metadata) if metadata.len() > 0 => {
                    found = true;
                    if previous
                        .as_ref()
                        .is_some_and(|(previous_path, size)| {
                            previous_path == path && *size == metadata.len()
                        })
                    {
                        return Ok(path.clone());
                    }
                    previous = Some((path.clone(), metadata.len()));
                }
                Ok(_) | Err(_) => {}
            }
        }
        if !found {
            previous = None;
        }
        thread::sleep(Duration::from_millis(250));
    }
    Err(CameraError::Timeout(format!(
        "capture file did not settle; expected one of {}",
        paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

fn shutter_ms(value: &str) -> Option<f64> {
    let value = value.trim().trim_matches('"').trim_end_matches('s').trim();
    let seconds = if let Some((numerator, denominator)) = value.split_once('/') {
        numerator.parse::<f64>().ok()? / denominator.parse::<f64>().ok()?
    } else { value.parse::<f64>().ok()? };
    let millis = seconds * 1000.0;
    (millis.is_finite() && millis > 0.0).then_some(millis)
}

fn select_shutter(supported: &str, requested_ms: f64) -> Result<String, CameraError> {
    useful_lines(supported).into_iter().find(|value| {
        shutter_ms(value).is_some_and(|actual| valid_exposure_ms(actual)
            && (actual - requested_ms).abs() <= requested_ms.max(1.0) * 1e-6)
    }).ok_or_else(|| CameraError::Safety(format!(
        "camera does not support {requested_ms} ms in its current shutter settings"
    )))
}

fn commit_when_released(
    staging: &Path,
    destination: &Path,
    timeout: Duration,
    cancel: Option<&AtomicBool>,
) -> Result<(), CameraError> {
    let deadline = Instant::now() + timeout;
    loop {
        check_capture_cancel(cancel)?;
        match fs::rename(staging, destination) {
            Ok(()) => return Ok(()),
            Err(error)
                if (matches!(
                    error.kind(),
                    std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::WouldBlock
                ) || matches!(error.raw_os_error(), Some(32 | 33)))
                    && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(250));
            }
            Err(error) => return Err(CameraError::Io(error.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_id_rejects_path_traversal() {
        assert!(validate_task_id("scan-001").is_ok());
        assert!(validate_task_id("../scan").is_err());
        assert!(validate_task_id(r"scan\child").is_err());
        assert!(validate_task_id("..").is_err());
    }

    #[test]
    fn parses_clean_remote_output() {
        let lines = useful_lines(
            "digiCamControl remote command line utility (2.1.7.0, date) running\r\n\r\n\"Save to PC only\"\r\n",
        );
        assert_eq!(lines, vec!["Save to PC only"]);
        assert_eq!(normalize(&lines[0]), "savetopconly");
    }

    #[test]
    fn uses_camera_exposure_enumeration_without_rounding() {
        let supported = "1/8000\n1/8\n1/5\n1.5s\n30s\nBulb\n";
        assert_eq!(select_shutter(supported, 0.125).unwrap(), "1/8000");
        assert_eq!(select_shutter(supported, 200.0).unwrap(), "1/5");
        assert_eq!(select_shutter(supported, 1500.0).unwrap(), "1.5s");
        assert_eq!(select_shutter(supported, 30000.0).unwrap(), "30s");
        assert!(select_shutter(supported, 120.0).is_err());
        assert!(!valid_exposure_ms(0.124));
        assert!(!valid_exposure_ms(30000.1));
        assert!(!valid_exposure_ms(f64::NAN));
        assert!(shutter_ms("Bulb").is_none());
        assert!(shutter_ms("1/0").is_none());
    }
    #[test]
    fn staging_paths_match_digicamcontrols_appended_nef() {
        let frames = Path::new("frames");
        let (capture_target, staging) =
            capture_staging_paths(frames, "camera-test-42.nef");
        assert_eq!(capture_target, frames.join(".camera-test-42.part"));
        assert_eq!(staging, frames.join(".camera-test-42.part.nef"));

        let (capture_target, staging) =
            capture_staging_paths(frames, "frame-0001.nef");
        assert_eq!(capture_target, frames.join(".frame-0001.part"));
        assert_eq!(staging, frames.join(".frame-0001.part.nef"));
        assert_eq!(
            capture_staging_candidates(frames, "frame-0001.nef"),
            vec![
                frames.join(".frame-0001.part.nef"),
                frames.join(".frame-0001.nef"),
            ]
        );
    }

    #[test]
    fn capture_waits_preserve_cancellation_reason() {
        let cancelled = AtomicBool::new(true);
        let absent = PathBuf::from("no-camera-file-in-this-offline-test.nef");
        assert!(matches!(
            wait_for_stable_file(&[absent.clone()], Duration::from_secs(1), Some(&cancelled)),
            Err(CameraError::Cancelled)
        ));
        assert!(matches!(
            commit_when_released(&absent, &absent, Duration::from_secs(1), Some(&cancelled)),
            Err(CameraError::Cancelled)
        ));
    }
}
