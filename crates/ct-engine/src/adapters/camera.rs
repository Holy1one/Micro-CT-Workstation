use serde::Serialize;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(60);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraHealth {
    pub connected: bool,
    pub executable: Option<String>,
    pub serial: Option<String>,
    pub transfer_policy: Option<String>,
    pub last_capture: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug)]
pub enum CameraError {
    Unavailable(String),
    Command(String),
    Safety(String),
    Io(String),
    Timeout(String),
}

impl std::fmt::Display for CameraError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
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

    pub fn set_exposure_ms(&mut self, exposure_ms: u32) -> Result<String, CameraError> {
        if !(1..=10_000).contains(&exposure_ms) {
            return Err(CameraError::Safety(
                "exposure must be within 1..10000 ms".into(),
            ));
        }
        if !self.health.connected {
            self.connect()?;
        }
        let shutter = shutter_speed_for_ms(exposure_ms);
        self.remote(
            &format!("set shutterspeed {shutter}"),
            COMMAND_TIMEOUT,
        )?;
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
        self.capture_named(save_path, task_id, &format!("camera-test-{stamp}.nef"))
    }

    pub fn capture_frame(
        &mut self,
        save_path: &Path,
        task_id: &str,
        index: u32,
    ) -> Result<PathBuf, CameraError> {
        self.capture_named(save_path, task_id, &format!("frame-{index:04}.nef"))
    }

    fn capture_named(
        &mut self,
        save_path: &Path,
        task_id: &str,
        filename: &str,
    ) -> Result<PathBuf, CameraError> {
        if !self.health.connected {
            self.connect()?;
        }
        validate_task_id(task_id)?;
        let transfer = self.remote("get transfer", COMMAND_TIMEOUT)?;
        let policy = useful_lines(&transfer.stdout).into_iter().last().unwrap_or_default();
        if normalize(&policy) != "savetopconly" {
            return self.fail(CameraError::Safety(
                "transfer policy changed; capture refused".into(),
            ));
        }

        let frames = save_path.join(task_id).join("frames");
        fs::create_dir_all(&frames).map_err(|error| CameraError::Io(error.to_string()))?;
        let destination = frames.join(filename);
        let staging = frames.join(format!(".{filename}.part"));
        if destination.exists() || staging.exists() {
            return Err(CameraError::Safety(
                "generated capture destination already exists".into(),
            ));
        }

        let command = format!("capture {}", staging.display());
        if let Err(error) = self.remote(&command, CAPTURE_TIMEOUT) {
            let _ = fs::remove_file(&staging);
            return self.fail(error);
        }
        if let Err(error) = wait_for_stable_file(&staging, CAPTURE_TIMEOUT) {
            let _ = fs::remove_file(&staging);
            return self.fail(error);
        }
        if let Err(error) = commit_when_released(&staging, &destination, CAPTURE_TIMEOUT) {
            let _ = fs::remove_file(&staging);
            return self.fail(error);
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
        let output = run_with_timeout(
            &self.executable,
            [
                OsString::from("/c"),
                OsString::from(command_text),
                OsString::from("/clean"),
            ],
            timeout,
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
) -> Result<CommandOutput, CameraError>
where
    I: IntoIterator<Item = OsString>,
{
    let mut child = Command::new(executable)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| CameraError::Unavailable(error.to_string()))?;
    let deadline = Instant::now() + timeout;
    loop {
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

fn wait_for_stable_file(path: &Path, timeout: Duration) -> Result<(), CameraError> {
    let deadline = Instant::now() + timeout;
    let mut previous_size = None;
    while Instant::now() < deadline {
        match fs::metadata(path) {
            Ok(metadata) if metadata.len() > 0 => {
                if previous_size == Some(metadata.len()) {
                    return Ok(());
                }
                previous_size = Some(metadata.len());
            }
            Ok(_) | Err(_) => previous_size = None,
        }
        thread::sleep(Duration::from_millis(250));
    }
    Err(CameraError::Timeout(format!(
        "capture file did not settle: {}",
        path.display()
    )))
}

fn shutter_speed_for_ms(exposure_ms: u32) -> String {
    if exposure_ms < 1_000 && 1_000 % exposure_ms == 0 {
        format!("1/{}", 1_000 / exposure_ms)
    } else {
        let seconds = f64::from(exposure_ms) / 1_000.0;
        let value = format!("{seconds:.3}");
        value.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

fn commit_when_released(
    staging: &Path,
    destination: &Path,
    timeout: Duration,
) -> Result<(), CameraError> {
    let deadline = Instant::now() + timeout;
    loop {
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
    fn converts_milliseconds_to_native_shutter_spelling() {
        assert_eq!(shutter_speed_for_ms(200), "1/5");
        assert_eq!(shutter_speed_for_ms(125), "1/8");
        assert_eq!(shutter_speed_for_ms(1500), "1.5");
    }
}
