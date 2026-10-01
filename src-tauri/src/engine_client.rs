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

/// Deletes engines extracted from earlier builds so the content-addressed
/// runtime cache cannot grow without bound. Only files that exactly match our
/// own `ct-engine-<16 hex>.exe|.tmp` naming are candidates, and the file in use
/// is never touched. Best effort: an engine still held by another running
/// instance cannot be deleted on Windows and is simply left in place.
fn prune_stale_engines(runtime_dir: &Path, keep: &Path) -> usize {
    let Ok(entries) = fs::read_dir(runtime_dir) else { return 0 };
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path == keep || !path.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(rest) = name.strip_prefix("ct-engine-") else { continue };
        let Some(stem) = rest.strip_suffix(".exe").or_else(|| rest.strip_suffix(".tmp")) else { continue };
        if stem.len() != 16 || !stem.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            continue;
        }
        if fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

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

    prune_stale_engines(&runtime_dir, &path);

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
        // Keep JSONL pipes and shutdown ownership without creating a Windows console.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
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
                // EOF lets ct-engine run its shutdown path after a slow request.
                // Killing it here would bypass the scan worker's beam-off cleanup.
                drop(self.stdin.take());
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
            "restore_previous" => Duration::from_secs(300),
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
        // 3. ct-engine may wait up to 65 seconds for an active scan worker to
        //    confirm cleanup after EOF. Keep the child alive through that
        //    window and the subsequent device OFF/disconnect transactions.
        let deadline = std::time::Instant::now() + Duration::from_secs(90);
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

    #[test]
    fn stale_extracted_engines_are_pruned_without_touching_foreign_files() {
        let root = std::env::temp_dir().join(format!("micro-ct-prune-engines-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let keep = root.join("ct-engine-0123456789abcdef.exe");
        let stale = root.join("ct-engine-fedcba9876543210.exe");
        let leftover = root.join("ct-engine-0011223344556677.tmp");
        let foreign = root.join("ct-engine-not-hex.exe");
        let unrelated = root.join("ct-workstation.exe");
        let directory = root.join("ct-engine-aabbccddeeff0011.exe");
        for path in [&keep, &stale, &leftover, &foreign, &unrelated] {
            fs::write(path, b"stub").unwrap();
        }
        fs::create_dir(&directory).unwrap();

        assert_eq!(prune_stale_engines(&root, &keep), 2, "only the stale engine and its leftover temp file may go");
        assert!(keep.is_file(), "the engine in use must survive");
        assert!(!stale.exists());
        assert!(!leftover.exists());
        assert!(foreign.is_file(), "names outside our own pattern must survive");
        assert!(unrelated.is_file(), "unrelated executables must survive");
        assert!(directory.is_dir(), "directories must survive");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn background_sidecar_exchanges_snapshots_and_exits_on_drop() {
        let path = embedded_engine_path().expect("embedded engine must extract");
        let mut engine = EngineClient::spawn_at(&path, true).expect("preview engine must start");
        let snapshot = engine.request("snapshot", serde_json::json!({})).expect("JSONL must work");
        assert!(snapshot.is_object());
        assert!(engine.child.try_wait().unwrap().is_none());
        let started = std::time::Instant::now();
        drop(engine);
        assert!(started.elapsed() < Duration::from_secs(8), "engine must shut down without kill timeout");
    }
}
