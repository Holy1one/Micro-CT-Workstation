//! Verified process and JSONL transport for the ct-engine sidecar.
//!
//! The client owns child-process handles, request sequencing, timeouts, and
//! executable verification. It treats protocol mismatches as hard failures and
//! contains no business rules; those live exclusively in ct-engine.
use ct_engine::{timestamp, Request, Response, PROTOCOL_VERSION};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

pub struct EngineClient {
    child: Child,
    stdin: Option<ChildStdin>,
    responses: Receiver<Result<Response, String>>,
    sequence: u64,
    response_sequence: u64,
    failed: bool,
}

const EMBEDDED_ENGINE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ct-engine.bin"));

fn embedded_engine_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let digest = Sha256::digest(EMBEDDED_ENGINE);
    let version = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let filename = if cfg!(windows) {
        format!("ct-engine-{version}.exe")
    } else {
        format!("ct-engine-{version}")
    };
    let runtime_dir = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("MicroCTWorkstation")
        .join("runtime");
    fs::create_dir_all(&runtime_dir)?;
    let path = runtime_dir.join(filename);

    let existing_is_current = fs::read(&path)
        .map(|bytes| Sha256::digest(bytes) == digest)
        .unwrap_or(false);
    if !existing_is_current {
        let temporary = runtime_dir.join(format!("ct-engine-{version}.tmp"));
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(EMBEDDED_ENGINE)?;
        file.sync_all()?;
        drop(file);
        if path.exists() {
            fs::remove_file(&path)?;
        }
        fs::rename(&temporary, &path)?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    }

    Ok(path)
}

impl EngineClient {
    pub fn spawn() -> Result<Self, Box<dyn std::error::Error>> {
        let path = embedded_engine_path()?;
        let preview = std::env::var_os("MICRO_CT_ENGINE_PREVIEW").is_some();
        Self::spawn_at(&path, preview)
    }

    fn spawn_at(path: &Path, preview: bool) -> Result<Self, Box<dyn std::error::Error>> {
        let mut command = Command::new(path);
        if preview {
            command.arg("--preview");
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| {
                format!(
                    "failed to launch ct-engine sidecar at {}: {error}",
                    path.display()
                )
            })?;
        let stdin = child.stdin.take().ok_or("Missing engine stdin")?;
        let stdout = child.stdout.take().ok_or("Missing engine stdout")?;
        let (sender, responses) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let response = line.map_err(|error| error.to_string()).and_then(|line| {
                    serde_json::from_str(&line).map_err(|error| error.to_string())
                });
                if sender.send(response).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            stdin: Some(stdin),
            responses,
            sequence: 0,
            response_sequence: 0,
            failed: false,
        })
    }

    pub fn request(&mut self, command: &str, payload: Value) -> Result<Value, String> {
        if self.failed {
            return Err("Engine transport failed; restart required".into());
        }
        let result = self.exchange(command, payload);
        if let Err(error) = &result {
            if !error.starts_with("ENGINE:") {
                self.failed = true;
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
        result
    }

    fn exchange(&mut self, command: &str, payload: Value) -> Result<Value, String> {
        self.sequence += 1;
        let request_id = format!("desktop-{}", self.sequence);
        let request = Request {
            protocol_version: PROTOCOL_VERSION,
            request_id: request_id.clone(),
            command: command.into(),
            payload,
            timestamp: timestamp(),
            sequence: self.sequence,
            error_code: None,
        };
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| "Engine stdin is closed".to_string())?;
        serde_json::to_writer(&mut *stdin, &request).map_err(|error| error.to_string())?;
        writeln!(&mut *stdin)
            .and_then(|_| stdin.flush())
            .map_err(|error| error.to_string())?;
        let timeout = match command {
            "home" => Duration::from_secs(310),
            "connect" => Duration::from_secs(30),
            "camera_test_capture" => Duration::from_secs(75),
            "preflight" | "retry_device" => Duration::from_secs(20),
            "xray_toggle" => Duration::from_secs(20),
            _ => Duration::from_secs(5),
        };
        let response = self
            .responses
            .recv_timeout(timeout)
            .map_err(|error| format!("Engine unavailable while handling {command}: {error}"))??;
        if response.protocol_version != PROTOCOL_VERSION
            || response.request_id != request_id
            || response.command != command
            || response.sequence <= self.response_sequence
        {
            return Err("Engine protocol mismatch".into());
        }
        self.response_sequence = response.sequence;
        if let Some(code) = response.error_code {
            return Err(format!("ENGINE:{code}"));
        }
        Ok(response.payload)
    }
}

impl Drop for EngineClient {
    fn drop(&mut self) {
        // 1. Ask the engine to latch a safe stop (it confirms Moxtek OFF).
        let _ = self.request("stop", serde_json::json!({"type":"stop"}));
        // 2. Closing stdin makes the engine run its deterministic shutdown
        //    (scan stop + Moxtek OFF + device release) even if the stop
        //    request itself failed.
        drop(self.stdin.take());
        // 3. Give the engine a bounded window to finish that cleanup; only a
        //    genuinely stuck process is killed.
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Ok(None) => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    return;
                }
                Err(_) => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_sidecar_reports_the_exact_path() {
        let missing = std::env::temp_dir().join("micro-ct-missing-sidecar.exe");
        let error = EngineClient::spawn_at(&missing, true)
            .err()
            .expect("missing sidecar must fail")
            .to_string();
        assert!(error.contains("failed to launch ct-engine sidecar at"));
        assert!(error.contains(&missing.display().to_string()));
    }
}
