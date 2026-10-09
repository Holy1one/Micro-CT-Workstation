//! Verified process and JSONL transport for the ct-engine sidecar.
//!
//! The client owns child-process handles, request sequencing, timeouts, and
//! executable verification. It treats protocol mismatches as hard failures and
//! contains no business rules; those live exclusively in ct-engine.
use ct_engine::{timestamp, Request, Response, PROTOCOL_VERSION};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::collections::HashMap;
use std::time::Duration;

#[derive(Clone)]
pub struct EngineClient { transport: Arc<Transport> }
struct Pending { command: String, reply: mpsc::Sender<Result<Value,String>> }
struct Transport {
    child: Mutex<Child>, stdin: Mutex<Option<ChildStdin>>,
    pending: Mutex<HashMap<String, Pending>>, session: Mutex<Option<String>>,
    sequence: AtomicU64, response_sequence: AtomicU64, failed: AtomicBool,
}

const ENGINE_SHA256: &str = include_str!(concat!(env!("OUT_DIR"), "/ct-engine.sha256"));

fn verify_engine_path(path: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let bytes = fs::read(path).map_err(|error| format!("Cannot read installed engine at {}: {error}. Repair the installation.", path.display()))?;
    if format!("{:x}", Sha256::digest(&bytes)) != ENGINE_SHA256 {
        return Err(format!("Installed engine integrity check failed at {}. Repair the installation.", path.display()).into());
    }
    Ok(path.to_path_buf())
}
fn installed_engine_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    #[cfg(not(test))]
    let directory = std::env::current_exe()?.parent().ok_or("Application directory unavailable")?.to_path_buf();
    #[cfg(test)]
    let directory = PathBuf::from(env!("OUT_DIR"));
    verify_engine_path(&directory.join(if cfg!(windows) { "ct-engine.exe" } else { "ct-engine" }))
}

impl EngineClient {
    /// Query the original child handle after confirmed exit. A recycled PID is
    /// not accepted as proof that this owned process shut down normally.
    pub fn exited_process_identity(&self) -> Result<(u32, u64, u32), String> {
        let mut child = self.transport.child.lock().map_err(|_| "Engine process handle poisoned")?;
        let status = child.try_wait().map_err(|error| error.to_string())?
            .ok_or("Owned engine has not exited")?;
        if !status.success() {return Err(format!("Engine exited with {status}; update refused"));}
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Foundation::FILETIME;
            use windows_sys::Win32::System::Threading::GetProcessTimes;
            let mut creation: FILETIME = unsafe {std::mem::zeroed()};
            let mut exit: FILETIME = unsafe {std::mem::zeroed()};
            let mut kernel: FILETIME = unsafe {std::mem::zeroed()};
            let mut user: FILETIME = unsafe {std::mem::zeroed()};
            if unsafe {GetProcessTimes(child.as_raw_handle(), &mut creation, &mut exit, &mut kernel, &mut user)} == 0 {
                return Err(format!("Owned engine creation time unavailable: {}", std::io::Error::last_os_error()));
            }
            let created = (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
            if created == 0 {return Err("Owned engine creation time is unknown".into());}
            Ok((child.id(), created, status.code().unwrap_or(0) as u32))
        }
        #[cfg(not(windows))]
        {Err("Verified Windows process identity is required for software installation".into())}
    }
    /// Shutdown ownership is explicit: retained request/preview clones cannot keep stdin open.
    pub fn begin_shutdown(&self){self.transport.fail("Engine is shutting down; stdin closed for safety cleanup");}
    pub fn wait_for_exit(&self,timeout:Duration)->Result<(),String>{
        let status=self.wait_for_shutdown(timeout)?;
        if status.success(){Ok(())}else{Err(format!("Engine exited with {status}; cleanup may be unconfirmed"))}
    }
    /// UI shutdown accepts an already exited child without declaring cleanup
    /// successful; installation keeps the strict wait_for_exit requirement.
    pub fn wait_for_shutdown(&self,timeout:Duration)->Result<std::process::ExitStatus,String>{
        let deadline=std::time::Instant::now()+timeout;
        loop{
            let status=self.transport.child.lock().map_err(|_|"Engine process handle poisoned")?.try_wait().map_err(|error|error.to_string())?;
            if let Some(status)=status{return Ok(status);}
            if std::time::Instant::now()>=deadline{return Err("Engine cleanup exit remains unconfirmed; process was not killed".into());}
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    pub fn spawn() -> Result<Self, Box<dyn std::error::Error>> {
        let path=installed_engine_path()?;
        Self::spawn_at(&path,std::env::var_os("MICRO_CT_ENGINE_PREVIEW").is_some())
    }
    fn spawn_at(path:&Path,preview:bool)->Result<Self,Box<dyn std::error::Error>> {
        let mut command=Command::new(path);
        #[cfg(test)]
        command.env("MICRO_CT_ENGINE_DATA_DIR",Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/desktop").join(format!("20261002-client-session-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos())));
        #[cfg(windows)] {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        if preview {command.arg("--preview");}
        let mut child=command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn()
            .map_err(|error|format!("failed to launch ct-engine sidecar at {}: {error}",path.display()))?;
        let stdin=child.stdin.take().ok_or("Missing engine stdin")?;
        let stdout=child.stdout.take().ok_or("Missing engine stdout")?;
        let transport=Arc::new(Transport {child:Mutex::new(child),stdin:Mutex::new(Some(stdin)),pending:Mutex::new(HashMap::new()),session:Mutex::new(None),sequence:AtomicU64::new(0),response_sequence:AtomicU64::new(0),failed:AtomicBool::new(false)});
        let weak=Arc::downgrade(&transport);
        std::thread::spawn(move|| {
            for line in BufReader::new(stdout).lines() {
                let Some(transport)=weak.upgrade() else {return;};
                let response=line.map_err(|e|e.to_string()).and_then(|line|serde_json::from_str::<Response>(&line).map_err(|e|e.to_string()));
                if let Err(error)=transport.deliver(response) {transport.fail(&error);return;}
            }
            if let Some(transport)=weak.upgrade() {transport.fail("Engine output closed");}
        });
        Ok(Self{transport})
    }
    /// Writing is serialized; waiting never holds the client, stdin or pending-map lock.
    pub fn prepare_software_update(&self) -> Result<Value,String> {
        self.request("prepare_software_update", serde_json::json!({"type":"prepare_software_update"}))
    }

    pub fn camera_capture_descriptor(&self, capture_id: &str) -> Result<Value,String> {
        self.request("camera_capture_descriptor", serde_json::json!({"type":"camera_capture_descriptor","captureId":capture_id}))
    }

    pub fn request(&self,command:&str,payload:Value)->Result<Value,String> {
        if self.transport.failed.load(Ordering::SeqCst){return Err("Engine transport failed; restart required".into());}
        let (reply,receive)=mpsc::channel();
        let request_id;
        {
            let mut input=self.transport.stdin.lock().map_err(|_|"Engine stdin poisoned")?;
            let stdin=input.as_mut().ok_or("Engine stdin is closed")?;
            let sequence=self.transport.sequence.fetch_add(1,Ordering::SeqCst)+1;
            request_id=format!("desktop-{sequence}");
            let session=self.transport.session.lock().map_err(|_|"Engine session poisoned")?.clone();
            if session.is_none()&&command!="snapshot" {return Err("Engine session handshake required".into());}
            let request=Request {engine_session_id:session,protocol_version:PROTOCOL_VERSION,request_id:request_id.clone(),command:command.into(),payload,timestamp:timestamp(),sequence,error_code:None};
            self.transport.pending.lock().map_err(|_|"Engine pending map poisoned")?.insert(request_id.clone(),Pending{command:command.into(),reply});
            let result=serde_json::to_writer(&mut *stdin,&request).map_err(|e|e.to_string())
                .and_then(|_|writeln!(&mut *stdin).and_then(|_|stdin.flush()).map_err(|e|e.to_string()));
            if let Err(error)=result {drop(input);self.transport.fail(&error);return Err(error);}
        }
        match receive.recv_timeout(Duration::from_secs(5)) {
            Ok(result)=>result,
            Err(error)=>{self.transport.fail("Engine response deadline exceeded; EOF safety cleanup requested");Err(format!("Engine unavailable while handling {command}: {error}"))}
        }
    }
}
impl Transport {
    fn deliver(&self,response:Result<Response,String>)->Result<(),String> {
        // Drain responses after explicit shutdown so closing pending requests
        // cannot turn a deliberate EOF cleanup into a broken stdout transport.
        if self.failed.load(Ordering::SeqCst){return Ok(());}
        let response=response?;
        if response.protocol_version!=PROTOCOL_VERSION || response.engine_session_id.is_empty()
            || response.sequence<=self.response_sequence.load(Ordering::SeqCst) {
            return Err("Engine protocol, session or sequence mismatch".into());
        }
        let pending=self.pending.lock().map_err(|_|"Engine pending map poisoned")?.remove(&response.request_id)
            .ok_or("Unexpected or duplicate engine response ID")?;
        if pending.command!=response.command {let _=pending.reply.send(Err("Engine command mismatch".into()));return Err("Engine command mismatch".into());}
        let mut session=self.session.lock().map_err(|_|"Engine session poisoned")?;
        match session.as_ref() {
            Some(expected) if expected!=&response.engine_session_id=> {let _=pending.reply.send(Err("Engine session changed".into()));return Err("Engine session changed".into());}
            None if response.command=="snapshot" && response.error_code.is_none()=>*session=Some(response.engine_session_id.clone()),
            None=>{let _=pending.reply.send(Err("Invalid session handshake".into()));return Err("Invalid session handshake".into());}
            _=>{},
        }
        if response.payload["engineSessionId"].as_str()!=Some(response.engine_session_id.as_str())
            || response.payload["snapshotSequence"].as_u64()!=Some(response.sequence) {
            let _=pending.reply.send(Err("Engine snapshot evidence mismatch".into()));return Err("Engine snapshot evidence mismatch".into());
        }
        self.response_sequence.store(response.sequence,Ordering::SeqCst);
        let result=match response.error_code {Some(code)=>Err(format!("ENGINE:{code}")),None=>Ok(response.payload)};
        let _=pending.reply.send(result);
        Ok(())
    }
    fn fail(&self,error:&str) {
        self.failed.store(true,Ordering::SeqCst);
        if let Ok(mut input)=self.stdin.lock(){drop(input.take());}
        if let Ok(mut pending)=self.pending.lock(){for (_,entry) in pending.drain(){let _=entry.reply.send(Err(error.into()));}}
    }
}
impl Drop for Transport {
    fn drop(&mut self) {
        // EOF reaches the independent reader immediately; the engine owns all safety cleanup.
        if let Ok(input)=self.stdin.get_mut(){drop(input.take());}
        let Ok(child)=self.child.get_mut()else{return;};
        let deadline=std::time::Instant::now()+Duration::from_secs(90);
        loop {match child.try_wait(){Ok(Some(_))=>return,Ok(None)if std::time::Instant::now()<deadline=>std::thread::sleep(Duration::from_millis(50)),_=>{eprintln!("Engine cleanup exit unconfirmed; process was not killed");return;}}}
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_child_exit_closes_desktop_but_cannot_authorize_an_update(){
        #[cfg(windows)]
        let mut command={let mut c=Command::new("cmd.exe");c.args(["/D","/C","exit 9"]);use std::os::windows::process::CommandExt;c.creation_flags(0x0800_0000);c};
        #[cfg(not(windows))]
        let mut command={let mut c=Command::new("sh");c.args(["-c","exit 9"]);c};
        let mut child=command.stdin(Stdio::piped()).stdout(Stdio::null()).spawn().unwrap();let stdin=child.stdin.take();
        let client=EngineClient{transport:Arc::new(Transport{child:Mutex::new(child),stdin:Mutex::new(stdin),pending:Mutex::new(HashMap::new()),session:Mutex::new(None),sequence:AtomicU64::new(0),response_sequence:AtomicU64::new(0),failed:AtomicBool::new(false)})};
        assert_eq!(client.wait_for_shutdown(Duration::from_secs(3)).unwrap().code(),Some(9));
        assert!(client.wait_for_exit(Duration::from_millis(50)).unwrap_err().contains("cleanup may be unconfirmed"));
        assert!(client.exited_process_identity().unwrap_err().contains("update refused"));
        assert_eq!(client.wait_for_shutdown(Duration::from_millis(50)).unwrap().code(),Some(9));
    }

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
    fn missing_and_modified_installed_engine_require_repair() {
        let directory=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/desktop")
            .join(format!("installed-engine-integrity-{}",std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path=directory.join("ct-engine.exe");
        let _=fs::remove_file(&path);
        assert!(verify_engine_path(&path).unwrap_err().to_string().contains("Repair the installation"));
        fs::write(&path,b"tampered executable").unwrap();
        assert!(verify_engine_path(&path).unwrap_err().to_string().contains("integrity check failed"));
        assert!(installed_engine_path().is_ok());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn background_sidecar_exchanges_snapshots_and_exits_on_drop() {
        let path = installed_engine_path().expect("installed engine must verify");
        let mut engine = EngineClient::spawn_at(&path, true).expect("preview engine must start");
        let snapshot = engine.request("snapshot", serde_json::json!({})).expect("JSONL must work");
        assert!(snapshot.is_object());
        assert!(engine.transport.child.lock().unwrap().try_wait().unwrap().is_none());
        let started = std::time::Instant::now();
        drop(engine);
        assert!(started.elapsed() < Duration::from_secs(8), "engine must shut down without kill timeout");
    }

    #[test]
    fn update_preparation_uses_the_real_versioned_command_envelope() {
        let path = installed_engine_path().unwrap();
        let engine = EngineClient::spawn_at(&path, true).unwrap();
        engine.request("snapshot", serde_json::json!({})).unwrap();
        let first = engine.prepare_software_update().expect("maintenance command must satisfy the sidecar's strict payload type gate");
        let operation = first["softwareUpdatePreparation"]["operationId"].as_str().unwrap().to_owned();
        let session = first["engineSessionId"].as_str().unwrap().to_owned();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let snapshot = engine.request("snapshot", serde_json::json!({})).unwrap();
            assert_eq!(snapshot["engineSessionId"], session);
            let report = &snapshot["softwareUpdatePreparation"];
            assert_eq!(report["operationId"], operation);
            if report["status"] == "ready" {
                assert_eq!(report["maintenanceLocked"], true);
                assert_eq!(report["noActiveTransactions"], true);
                assert!(snapshot["operations"].as_array().unwrap().iter().any(|op|op["operationId"] == operation && op["status"] == "completed"));
                break;
            }
            assert!(std::time::Instant::now() < deadline, "real sidecar did not confirm maintenance readiness");
            std::thread::sleep(Duration::from_millis(20));
        }
        engine.begin_shutdown();
        engine.wait_for_exit(Duration::from_secs(10)).unwrap();
        assert_eq!(engine.exited_process_identity().unwrap().2, 0);
    }

    #[test]
    fn capture_descriptor_uses_the_real_versioned_command_envelope() {
        let path = installed_engine_path().unwrap();
        let engine = EngineClient::spawn_at(&path, true).unwrap();
        engine.request("snapshot", serde_json::json!({})).unwrap();
        let error = engine.camera_capture_descriptor("unregistered-output").expect_err("lookup cannot invent a registered file");
        assert_eq!(error, "ENGINE:CAPTURE_NOT_REGISTERED", "typed lookup must reach the real registration gate, not fail the command envelope gate");
        engine.begin_shutdown();
        engine.wait_for_exit(Duration::from_secs(10)).unwrap();
        assert_eq!(engine.exited_process_identity().unwrap().2, 0);
    }

    #[test]
    fn explicit_shutdown_closes_stdin_and_wakes_pending_requests_with_live_client_clones(){
        let path=installed_engine_path().unwrap();let engine=EngineClient::spawn_at(&path,true).unwrap();
        engine.request("snapshot",serde_json::json!({})).unwrap();let retained=engine.clone();
        let (reply,receive)=mpsc::channel();engine.transport.pending.lock().unwrap().insert("held-request".into(),Pending{command:"snapshot".into(),reply});
        engine.begin_shutdown();assert!(receive.recv_timeout(Duration::from_secs(1)).unwrap().is_err());
        assert!(retained.request("snapshot",serde_json::json!({})).is_err());
        engine.wait_for_exit(Duration::from_secs(3)).expect("retained clones must not prevent EOF cleanup exit");
        assert!(retained.transport.child.lock().unwrap().try_wait().unwrap().is_some());
    }

    #[test]
    fn sidecar_eof_cleanup_cannot_be_blocked_by_an_unread_stdout_pipe() {
        let path=installed_engine_path().unwrap();
        let mut command=Command::new(path);
        #[cfg(windows)]{use std::os::windows::process::CommandExt;command.creation_flags(0x0800_0000);}
        let mut child=command.arg("--preview")
            .env("MICRO_CT_ENGINE_DATA_DIR",Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/desktop/20261002-stdout-backpressure"))
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
        let mut input=child.stdin.take().unwrap();
        for sequence in 1..=24 {
            let request=Request{engine_session_id:None,protocol_version:PROTOCOL_VERSION,request_id:format!("backpressure-{sequence}"),command:"snapshot".into(),payload:serde_json::json!({}),timestamp:timestamp(),sequence,error_code:None};
            if serde_json::to_writer(&mut input,&request).is_err(){break;}if writeln!(&mut input).is_err(){break;}
            if sequence==1{let _=input.flush();std::thread::sleep(Duration::from_millis(100));}
        }
        let _=input.flush();std::thread::sleep(Duration::from_millis(100));drop(input);
        // Deliberately keep ChildStdout open without reading its bytes.
        let deadline=std::time::Instant::now()+Duration::from_secs(3);
        loop {
            if child.try_wait().unwrap().is_some(){break;}
            if std::time::Instant::now()>=deadline{let _=child.kill();let _=child.wait();panic!("EOF must reach the domain while stdout consumer is stalled");}
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
