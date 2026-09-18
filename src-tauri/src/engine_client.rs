//! Process/IPC ownership only. Business rules live exclusively in ct-engine.
use ct_engine::{timestamp, Request, Response, PROTOCOL_VERSION};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

pub struct EngineClient {
    child: Child,
    stdin: ChildStdin,
    responses: Receiver<Result<Response, String>>,
    sequence: u64,
    response_sequence: u64,
    failed: bool,
}

impl EngineClient {
    pub fn spawn() -> Result<Self, Box<dyn std::error::Error>> {
        let exe = std::env::current_exe()?;
        let filename = if cfg!(windows) {
            "ct-engine.exe"
        } else {
            "ct-engine"
        };
        let path: PathBuf = exe
            .parent()
            .ok_or("Missing executable directory")?
            .join(filename);
        if !path.is_file() {
            return Err(format!(
                "ct-engine sidecar is missing next to the workstation executable: {}",
                path.display()
            )
            .into());
        }
        Self::spawn_at(&path, cfg!(debug_assertions))
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
            stdin,
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
        serde_json::to_writer(&mut self.stdin, &request).map_err(|error| error.to_string())?;
        writeln!(&mut self.stdin)
            .and_then(|_| self.stdin.flush())
            .map_err(|error| error.to_string())?;
        let response = self
            .responses
            .recv_timeout(Duration::from_secs(2))
            .map_err(|error| format!("Engine unavailable: {error}"))??;
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
        let _ = self.request("stop", serde_json::json!({}));
        let _ = self.child.kill();
        let _ = self.child.wait();
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
