//! Bounded JSONL ingress/egress. EOF and safety admission remain independent from slow clients.
use ct_engine::{timestamp,Engine,Request,Response,PROTOCOL_VERSION};
use serde_json::json;
use std::io::{self,BufRead,Write};
use std::sync::{mpsc,Arc};
use std::sync::atomic::{AtomicBool,Ordering};
use std::time::Duration;
const MAX_REQUEST_BYTES:usize=2*1024*1024+65536;

#[cfg(windows)]
fn independent_stdout()->io::Result<std::fs::File>{
    use std::os::windows::io::AsHandle;
    Ok(std::fs::File::from(io::stdout().as_handle().try_clone_to_owned()?))
}
#[cfg(unix)]
fn independent_stdout()->io::Result<std::fs::File>{
    use std::os::fd::AsFd;
    Ok(std::fs::File::from(io::stdout().as_fd().try_clone_to_owned()?))
}

fn main()->io::Result<()> {
    let preview=std::env::args().skip(1).any(|arg|arg=="--preview");
    let mut engine=Engine::new(preview);
    let (normal_send,normal_receive)=mpsc::sync_channel::<Request>(32);
    let (safety_send,safety_receive)=mpsc::sync_channel::<Request>(32);
    let (rejected_send,rejected_receive)=mpsc::sync_channel::<Option<Request>>(32);
    let eof=Arc::new(AtomicBool::new(false));let ingress_failed=Arc::new(AtomicBool::new(false));
    let reader_eof=eof.clone();let reader_failure=ingress_failed.clone();
    std::thread::spawn(move|| {
        let stdin=io::stdin();let mut reader=stdin.lock();
        loop {
            let mut line=Vec::new();let mut oversized=false;
            loop {
                let Ok(buffer)=reader.fill_buf()else{reader_eof.store(true,Ordering::SeqCst);return;};
                if buffer.is_empty(){reader_eof.store(true,Ordering::SeqCst);return;}
                let newline=buffer.iter().position(|byte|*byte==b'\n');let count=newline.map_or(buffer.len(),|position|position+1);
                if line.len()+count>MAX_REQUEST_BYTES{oversized=true;line.clear();}
                if !oversized{line.extend_from_slice(&buffer[..count]);}
                reader.consume(count);if newline.is_some(){break;}
            }
            let request=if oversized{None}else{serde_json::from_slice::<Request>(&line).ok()};
            let accepted=match request {
                Some(request)if matches!(request.command.as_str(),"stop"|"xray_off"|"disconnect"|"abandon_scan")=>safety_send.try_send(request).is_ok(),
                Some(request)=>match normal_send.try_send(request){Ok(())=>true,Err(mpsc::TrySendError::Full(mut rejected))=>{rejected.error_code=Some("COMMAND_QUEUE_FULL".into());rejected_send.try_send(Some(rejected)).is_ok()},Err(_)=>false},
                None=>rejected_send.try_send(None).is_ok(),
            };
            if !accepted{reader_failure.store(true,Ordering::SeqCst);return;}
        }
    });
    let (response_send,response_receive)=mpsc::sync_channel::<Response>(16);
    let egress_failed=Arc::new(AtomicBool::new(false));let writer_failure=egress_failed.clone();
    std::thread::spawn(move|| {
        let output=match independent_stdout(){Ok(output)=>output,Err(_)=>{writer_failure.store(true,Ordering::SeqCst);return;}};
        let mut writer=io::BufWriter::new(output);
        for response in response_receive {
            let result=serde_json::to_writer(&mut writer,&response).map_err(io::Error::other)
                .and_then(|_|writeln!(&mut writer)).and_then(|_|writer.flush());
            if result.is_err(){writer_failure.store(true,Ordering::SeqCst);return;}
        }
    });
    let mut failed=false;
    while !eof.load(Ordering::SeqCst) {
        if ingress_failed.load(Ordering::SeqCst)||egress_failed.load(Ordering::SeqCst){failed=true;break;}
        engine.tick();
        let request=if let Ok(safety)=safety_receive.try_recv(){Some(Some(safety))}
            else if let Ok(normal)=normal_receive.try_recv(){Some(Some(normal))}
            else if let Ok(rejected)=rejected_receive.try_recv(){Some(rejected)}
            else {std::thread::sleep(Duration::from_millis(10));None};
        let Some(request)=request else{continue;};
        let response=match request {
            Some(request)=>engine.handle(request),
            None=>Response{engine_session_id:engine.snapshot()["engineSessionId"].as_str().unwrap_or_default().into(),protocol_version:PROTOCOL_VERSION,request_id:String::new(),command:"invalid".into(),payload:json!({"message":"Expected a versioned JSONL envelope within 2 MiB + 64 KiB"}),timestamp:timestamp(),sequence:0,error_code:Some("INVALID_ENVELOPE".into())},
        };
        if response_send.try_send(response).is_err(){failed=true;break;}
    }
    // A blocked stdout writer is never joined; the process exits after the domain owns cleanup.
    let shutdown=engine.shutdown();drop(response_send);
    shutdown_exit_result(failed,shutdown)
}

fn shutdown_exit_result(transport_failed:bool,shutdown:Result<(),String>)->io::Result<()> {
    match (transport_failed,shutdown) {
        (false,Ok(()))=>Ok(()),
        (true,Ok(()))=>Err(io::Error::other("Control transport unavailable or bounded queue exhausted; safety cleanup confirmed")),
        (false,Err(error))=>Err(io::Error::other(format!("EOF safety cleanup unconfirmed: {error}"))),
        (true,Err(error))=>Err(io::Error::other(format!("Control transport unavailable or bounded queue exhausted; safety cleanup unconfirmed: {error}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{ExitCode,Termination};
    #[test]
    fn normal_eof_cannot_hide_unconfirmed_shutdown() {
        let result=shutdown_exit_result(false,Err("Camera cleanup unconfirmed; Nano STOP unconfirmed".into()));
        let error=result.as_ref().expect_err("an EOF shutdown failure must remain a process failure").to_string();
        assert!(error.contains("EOF safety cleanup unconfirmed"));
        assert!(error.contains("Camera cleanup unconfirmed"));
        assert!(error.contains("Nano STOP unconfirmed"));
        assert_eq!(result.report(),ExitCode::FAILURE);
    }
    #[test]
    fn confirmed_shutdown_preserves_normal_and_failed_transport_exit_results() {
        assert_eq!(shutdown_exit_result(false,Ok(())).report(),ExitCode::SUCCESS);
        assert!(shutdown_exit_result(true,Ok(())).unwrap_err().to_string().contains("cleanup confirmed"));
        let error=shutdown_exit_result(true,Err("OFF unconfirmed".into())).unwrap_err().to_string();
        assert!(error.contains("Control transport unavailable"));assert!(error.contains("OFF unconfirmed"));
    }
}
