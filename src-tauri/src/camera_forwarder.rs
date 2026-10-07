//! Thin, bounded Live View pipe reader. Only an engine-issued local endpoint is
//! accepted. The webview supplies a sequence number and never a file/pipe path;
//! JPEG validation is shared with the engine and no device state is owned here.

use ct_engine::devices::camera::{
    decode_live_view_frame, encode_live_view_ack, LiveViewFrame, LiveViewPipeDescriptor, LIVE_VIEW_HEADER_BYTES, LIVE_VIEW_PIPE_VERSION,
    MAX_LIVE_VIEW_BYTES,
};
use serde_json::{json, Value};

fn validate_descriptor(descriptor: &LiveViewPipeDescriptor) -> Result<(), String> {
    let suffix = descriptor
        .endpoint
        .strip_prefix(r"\\.\pipe\microct-lv-")
        .ok_or("Foreign Live View pipe endpoint")?;
    if suffix.len() != 32
        || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
        || descriptor.session_id.len() != 32
        || !descriptor
            .session_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || descriptor.version != LIVE_VIEW_PIPE_VERSION
        || descriptor.max_frame_bytes != MAX_LIVE_VIEW_BYTES
    {
        return Err("Invalid engine Live View descriptor".into());
    }
    Ok(())
}

pub fn read_frame(
    descriptor: &LiveViewPipeDescriptor,
    after_sequence: u64,
) -> Result<Value, String> {
    validate_descriptor(descriptor)?;
    let Some(frame) = read_pipe(descriptor,after_sequence)? else {
        // No connection became available in this polling window. The caller
        // still revalidates the engine session before forwarding this null.
        return Ok(Value::Null);
    };
    Ok(
        json!({"sessionId":frame.session_id,"sequence":frame.sequence,"width":frame.width,"height":frame.height,"capturedAtMs":frame.captured_at_ms,"jpeg":frame.jpeg}),
    )
}

#[cfg(not(windows))]
fn read_pipe(_descriptor: &LiveViewPipeDescriptor,_after_sequence:u64) -> Result<Option<LiveViewFrame>, String> {
    Err("Windows Live View pipe is unavailable".into())
}

#[cfg(windows)]
fn read_pipe(descriptor:&LiveViewPipeDescriptor,after_sequence:u64) -> Result<Option<LiveViewFrame>, String> {
    read_pipe_delayed(descriptor,after_sequence,std::time::Duration::ZERO)
}

#[cfg(windows)]
fn read_pipe_delayed(descriptor:&LiveViewPipeDescriptor,after_sequence:u64,initial_read_delay:std::time::Duration) -> Result<Option<LiveViewFrame>, String> {
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::*;
    use windows_sys::Win32::Storage::FileSystem::*;
    use windows_sys::Win32::System::Pipes::WaitNamedPipeW;
    use windows_sys::Win32::System::Threading::*;
    use windows_sys::Win32::System::IO::*;
    struct OwnedHandle(HANDLE);
    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    let path: Vec<u16> = descriptor.endpoint.encode_utf16().chain([0]).collect();
    let deadline = Instant::now() + Duration::from_millis(250);
    unsafe {
        // The producer replaces its single instance after each receipt. Only
        // missing/busy connection errors are transient; once connected, every
        // read, validation and ACK must finish within this same deadline.
        let raw = loop {
            if Instant::now() >= deadline {
                return Ok(None);
            }
            let raw = CreateFileW(
                path.as_ptr(), GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE, std::ptr::null(),
                OPEN_EXISTING, FILE_FLAG_OVERLAPPED, std::ptr::null_mut(),
            );
            if raw != INVALID_HANDLE_VALUE {
                break raw;
            }
            let code = GetLastError();
            if !matches!(code, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND | ERROR_PIPE_BUSY) {
                return Err(format!("Live View endpoint unavailable: Win32 {code} ({})", std::io::Error::from_raw_os_error(code as i32)));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            if code == ERROR_PIPE_BUSY && WaitNamedPipeW(path.as_ptr(), remaining.as_millis().min(20).max(1) as u32) == 0 {
                let code = GetLastError();
                if !matches!(code, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND | ERROR_PIPE_BUSY | ERROR_SEM_TIMEOUT) {
                    return Err(format!("Live View endpoint wait failed: Win32 {code} ({})", std::io::Error::from_raw_os_error(code as i32)));
                }
            }
            // WaitNamedPipe returns immediately when no instance exists. A
            // bounded pause also prevents a busy loop if another client wins.
            std::thread::sleep(deadline.saturating_duration_since(Instant::now()).min(Duration::from_millis(5)));
        };
        let handle = OwnedHandle(raw);
        let event_raw = CreateEventW(std::ptr::null(), 1, 0, std::ptr::null());
        if event_raw.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let event = OwnedHandle(event_raw);
        // The production caller uses ZERO; the pipe regression delays a consumer
        // after connection while the shared 250 ms deadline continues to run.
        if !initial_read_delay.is_zero(){std::thread::sleep(initial_read_delay);}
        let read_exact = |buffer: &mut [u8]| -> Result<(), String> {
            let mut offset = 0;
            while offset < buffer.len() {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err("Live View read deadline exceeded".into());
                }
                ResetEvent(event.0);
                let mut overlapped: OVERLAPPED = std::mem::zeroed();
                overlapped.hEvent = event.0;
                let mut received = 0;
                let ok = ReadFile(
                    handle.0,
                    buffer[offset..].as_mut_ptr(),
                    (buffer.len() - offset) as u32,
                    &mut received,
                    &mut overlapped,
                );
                if ok == 0 {
                    if GetLastError() != ERROR_IO_PENDING {
                        return Err(format!(
                            "Incomplete Live View frame: {}",
                            std::io::Error::last_os_error()
                        ));
                    }
                    if WaitForSingleObject(event.0, remaining.as_millis().max(1) as u32)
                        != WAIT_OBJECT_0
                    {
                        CancelIoEx(handle.0, &overlapped);
                        // The OVERLAPPED and destination buffer stay alive until
                        // local pipe cancellation has completed.
                        GetOverlappedResult(handle.0, &overlapped, &mut received, 1);
                        return Err("Live View pipe consumer deadline exceeded".into());
                    }
                    if GetOverlappedResult(handle.0, &overlapped, &mut received, 0) == 0 {
                        return Err(std::io::Error::last_os_error().to_string());
                    }
                }
                if received == 0 {
                    return Err("Truncated Live View frame".into());
                }
                offset += received as usize;
            }
            Ok(())
        };
        let mut header = vec![0; LIVE_VIEW_HEADER_BYTES];
        read_exact(&mut header)?;
        let length = u32::from_le_bytes(header[48..52].try_into().unwrap()) as usize;
        if !(4..=MAX_LIVE_VIEW_BYTES).contains(&length) {
            return Err("Oversized or empty Live View frame".into());
        }
        let mut bytes = header;
        bytes.resize(LIVE_VIEW_HEADER_BYTES + length, 0);
        read_exact(&mut bytes[LIVE_VIEW_HEADER_BYTES..])?;
        let frame=decode_live_view_frame(&bytes,&descriptor.session_id,after_sequence).map_err(|error|error.to_string())?;
        let ack=encode_live_view_ack(&frame.session_id,frame.sequence).map_err(|error|error.to_string())?;
        let mut offset=0;
        while offset<ack.len(){
            let remaining=deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero(){return Err("Live View validation/ACK deadline exceeded".into());}
            ResetEvent(event.0);
            let mut overlapped:OVERLAPPED=std::mem::zeroed();overlapped.hEvent=event.0;
            let mut written=0;
            let ok=WriteFile(handle.0,ack[offset..].as_ptr(),(ack.len()-offset)as u32,&mut written,&mut overlapped);
            if ok==0{
                if GetLastError()!=ERROR_IO_PENDING{return Err(format!("Live View ACK write failed: {}",std::io::Error::last_os_error()));}
                if WaitForSingleObject(event.0,remaining.as_millis().max(1)as u32)!=WAIT_OBJECT_0{
                    CancelIoEx(handle.0,&overlapped);GetOverlappedResult(handle.0,&overlapped,&mut written,1);
                    return Err("Live View ACK consumer deadline exceeded".into());
                }
                if GetOverlappedResult(handle.0,&overlapped,&mut written,0)==0{return Err(std::io::Error::last_os_error().to_string());}
            }
            if written==0{return Err("Live View ACK write made no progress".into());}
            offset+=written as usize;
        }
        Ok(Some(frame))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_random_local_engine_descriptors_are_accepted() {
        let mut descriptor = LiveViewPipeDescriptor {
            endpoint: format!(r"\\.\pipe\microct-lv-{}", "a".repeat(32)),
            session_id: "b".repeat(32),
            version: LIVE_VIEW_PIPE_VERSION,
            max_frame_bytes: MAX_LIVE_VIEW_BYTES,
        };
        validate_descriptor(&descriptor).unwrap();
        descriptor.endpoint = r"\\remote\pipe\microct-lv-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into();
        assert!(validate_descriptor(&descriptor).is_err());
        descriptor.endpoint = r"C:\private.json".into();
        assert!(validate_descriptor(&descriptor).is_err());
        descriptor.endpoint = format!(r"\\.\pipe\microct-lv-{}", "a".repeat(32));
        descriptor.max_frame_bytes = usize::MAX;
        assert!(validate_descriptor(&descriptor).is_err());
    }

    #[cfg(windows)]
    fn synthetic_jpeg()->Vec<u8>{
        // Generated 8x6 uniform-gray JPEG; no camera or user image is involved.
        let text="/9j/4AAQSkZJRgABAQEAYABgAAD/2wBDAAMCAgMCAgMDAwMEAwMEBQgFBQQEBQoHBwYIDAoMDAsKCwsNDhIQDQ4RDgsLEBYQERMUFRUVDA8XGBYUGBIUFRT/2wBDAQMEBAUEBQkFBQkUDQsNFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBT/wAARCAAGAAgDASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVYnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwD5/ooooA//2Q==";
        let alphabet=b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";let(mut acc,mut bits)=(0u32,0usize);let mut bytes=Vec::new();
        for symbol in text.bytes().take_while(|byte|*byte!=b'='){let value=alphabet.iter().position(|byte|*byte==symbol).unwrap()as u32;acc=(acc<<6)|value;bits+=6;if bits>=8{bits-=8;bytes.push((acc>>bits)as u8);}}
        bytes
    }
    #[cfg(windows)]
    fn local_descriptor(session: &str) -> LiveViewPipeDescriptor {
        // Parallel fixtures can observe the same Windows clock tick. Process
        // identity and a counter reserve distinct names without clock entropy.
        static NEXT_PIPE_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = NEXT_PIPE_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        LiveViewPipeDescriptor {
            endpoint: format!(r"\\.\pipe\microct-lv-{:016x}{sequence:016x}", u64::from(std::process::id())),
            session_id: session.into(), version: LIVE_VIEW_PIPE_VERSION,
            max_frame_bytes: MAX_LIVE_VIEW_BYTES,
        }
    }
    #[cfg(windows)]
    fn local_pipe(endpoint: &str, access: u32) -> std::fs::File {
        use std::os::windows::io::FromRawHandle;
        use windows_sys::Win32::Foundation::*;
        use windows_sys::Win32::System::Pipes::*;
        unsafe {
            let path: Vec<u16> = endpoint.encode_utf16().chain([0]).collect();
            let raw = CreateNamedPipeW(path.as_ptr(), access, PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, 1, (MAX_LIVE_VIEW_BYTES + LIVE_VIEW_HEADER_BYTES) as u32, 32, 250, std::ptr::null());
            if raw == INVALID_HANDLE_VALUE {
                let code = GetLastError();
                panic!("CreateNamedPipeW {endpoint} failed: Win32 {code} ({})", std::io::Error::from_raw_os_error(code as i32));
            }
            std::fs::File::from_raw_handle(raw)
        }
    }
    #[cfg(windows)]
    enum PeerAvailability {
        Ready,
        Gap(std::time::Duration),
        Busy(std::time::Duration),
    }
    #[cfg(windows)]
    fn local_peer(bytes:Vec<u8>,session:&str,payload_delay:std::time::Duration)->(LiveViewPipeDescriptor,std::thread::JoinHandle<Option<[u8;32]>>){
        local_peer_with_availability(bytes, session, payload_delay, PeerAvailability::Ready)
    }
    #[cfg(windows)]
    fn local_peer_with_availability(bytes:Vec<u8>,session:&str,payload_delay:std::time::Duration,availability:PeerAvailability)->(LiveViewPipeDescriptor,std::thread::JoinHandle<Option<[u8;32]>>){
        use std::io::{Read,Write};use std::os::windows::io::{AsRawHandle,FromRawHandle};
        use windows_sys::Win32::Foundation::*;use windows_sys::Win32::Storage::FileSystem::*;use windows_sys::Win32::System::Pipes::*;
        let descriptor = local_descriptor(session);
        let endpoint = descriptor.endpoint.clone();
        let(ready,receive)=std::sync::mpsc::channel();
        let thread=std::thread::spawn(move||unsafe{
            let mut file = match availability {
                PeerAvailability::Ready => {
                    let file = local_pipe(&endpoint, PIPE_ACCESS_DUPLEX);
                    ready.send(()).unwrap();
                    file
                }
                PeerAvailability::Gap(delay) => {
                    // Publish the descriptor before any instance exists, just as
                    // a producer instance replacement leaves a temporary gap.
                    ready.send(()).unwrap();
                    std::thread::sleep(delay);
                    local_pipe(&endpoint, PIPE_ACCESS_DUPLEX)
                }
                PeerAvailability::Busy(delay) => {
                    let occupied = local_pipe(&endpoint, PIPE_ACCESS_DUPLEX);
                    let path: Vec<u16> = endpoint.encode_utf16().chain([0]).collect();
                    let client_raw = CreateFileW(path.as_ptr(), GENERIC_READ | GENERIC_WRITE, 0, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut());
                    assert_ne!(client_raw, INVALID_HANDLE_VALUE);
                    let client = std::fs::File::from_raw_handle(client_raw);
                    ready.send(()).unwrap();
                    std::thread::sleep(delay);
                    drop(client);
                    drop(occupied);
                    local_pipe(&endpoint, PIPE_ACCESS_DUPLEX)
                }
            };
            let raw = file.as_raw_handle();
            let connected=ConnectNamedPipe(raw,std::ptr::null_mut());assert!(connected!=0||GetLastError()==ERROR_PIPE_CONNECTED);
            file.write_all(&bytes[..LIVE_VIEW_HEADER_BYTES]).unwrap();std::thread::sleep(payload_delay);file.write_all(&bytes[LIVE_VIEW_HEADER_BYTES..]).unwrap();
            let mut ack=[0;32];if file.read_exact(&mut ack).is_ok(){Some(ack)}else{None}
        });
        receive.recv_timeout(std::time::Duration::from_secs(2)).unwrap();(descriptor,thread)
    }
    #[cfg(windows)]
    #[test]
    fn complete_frame_is_validated_then_acknowledged_over_a_delayed_local_duplex_pipe(){
        let session="00112233445566778899aabbccddeeff";
        let frame=LiveViewFrame{session_id:session.into(),sequence:7,width:8,height:6,captured_at_ms:123,jpeg:synthetic_jpeg()};
        let bytes=ct_engine::devices::camera::encode_live_view_frame(&frame).unwrap();
        let(descriptor,peer)=local_peer(bytes,session,std::time::Duration::from_millis(80));
        let started=std::time::Instant::now();let received=read_frame(&descriptor,6).unwrap();
        assert_eq!(received["sequence"],7);assert_eq!(received["width"],8);assert_eq!(received["height"],6);
        assert!(started.elapsed()>=std::time::Duration::from_millis(70));assert!(started.elapsed()<std::time::Duration::from_millis(250));
        assert_eq!(peer.join().unwrap().unwrap(),encode_live_view_ack(session,7).unwrap());
    }
    #[cfg(windows)]
    #[test]
    fn rejected_stale_frame_closes_the_pipe_without_acknowledgement(){
        let session="00112233445566778899aabbccddeeff";let frame=LiveViewFrame{session_id:session.into(),sequence:7,width:8,height:6,captured_at_ms:123,jpeg:synthetic_jpeg()};
        let bytes=ct_engine::devices::camera::encode_live_view_frame(&frame).unwrap();let(descriptor,peer)=local_peer(bytes,session,std::time::Duration::ZERO);
        assert!(read_frame(&descriptor,7).is_err());assert!(peer.join().unwrap().is_none());
    }
    #[cfg(windows)]
    #[test]
    fn slow_connected_consumer_still_receives_all_bytes_before_acknowledgement(){
        let session="00112233445566778899aabbccddeeff";let frame=LiveViewFrame{session_id:session.into(),sequence:9,width:8,height:6,captured_at_ms:123,jpeg:synthetic_jpeg()};
        let bytes=ct_engine::devices::camera::encode_live_view_frame(&frame).unwrap();let(descriptor,peer)=local_peer(bytes,session,std::time::Duration::ZERO);
        let started=std::time::Instant::now();let received=read_pipe_delayed(&descriptor,8,std::time::Duration::from_millis(90)).unwrap().unwrap();
        assert_eq!(received.jpeg,frame.jpeg);assert!(started.elapsed()>=std::time::Duration::from_millis(80));assert!(started.elapsed()<std::time::Duration::from_millis(250));
        assert_eq!(peer.join().unwrap().unwrap(),encode_live_view_ack(session,9).unwrap());
    }
    #[cfg(windows)]
    #[test]
    fn producer_endpoint_gap_is_retried_within_the_frame_deadline() {
        let session = "00112233445566778899aabbccddeeff";
        let frame = LiveViewFrame { session_id: session.into(), sequence: 10, width: 8, height: 6, captured_at_ms: 123, jpeg: synthetic_jpeg() };
        let bytes = ct_engine::devices::camera::encode_live_view_frame(&frame).unwrap();
        let (descriptor, peer) = local_peer_with_availability(bytes, session, std::time::Duration::ZERO, PeerAvailability::Gap(std::time::Duration::from_millis(70)));
        let started = std::time::Instant::now();
        let received = read_frame(&descriptor, 9).unwrap();
        assert_eq!(received["sequence"], 10);
        assert!(started.elapsed() >= std::time::Duration::from_millis(60));
        assert!(started.elapsed() < std::time::Duration::from_millis(250));
        assert_eq!(peer.join().unwrap().unwrap(), encode_live_view_ack(session, 10).unwrap());
    }
    #[cfg(windows)]
    #[test]
    fn occupied_producer_instance_is_retried_after_its_client_releases() {
        let session = "00112233445566778899aabbccddeeff";
        let frame = LiveViewFrame { session_id: session.into(), sequence: 11, width: 8, height: 6, captured_at_ms: 123, jpeg: synthetic_jpeg() };
        let bytes = ct_engine::devices::camera::encode_live_view_frame(&frame).unwrap();
        let (descriptor, peer) = local_peer_with_availability(bytes, session, std::time::Duration::ZERO, PeerAvailability::Busy(std::time::Duration::from_millis(100)));
        let started = std::time::Instant::now();
        let received = read_frame(&descriptor, 10).unwrap();
        assert_eq!(received["sequence"], 11);
        assert!(started.elapsed() >= std::time::Duration::from_millis(90));
        assert!(started.elapsed() < std::time::Duration::from_millis(250));
        assert_eq!(peer.join().unwrap().unwrap(), encode_live_view_ack(session, 11).unwrap());
    }
    #[cfg(windows)]
    #[test]
    fn no_producer_instance_returns_null_after_the_bounded_connection_window() {
        let descriptor = local_descriptor("00112233445566778899aabbccddeeff");
        let started = std::time::Instant::now();
        assert!(read_frame(&descriptor, 0).unwrap().is_null());
        assert!(started.elapsed() >= std::time::Duration::from_millis(240));
        assert!(started.elapsed() < std::time::Duration::from_millis(350));
    }
    #[cfg(windows)]
    #[test]
    fn occupied_instance_returns_null_after_the_same_bounded_connection_window() {
        use std::os::windows::io::FromRawHandle;
        use windows_sys::Win32::Foundation::*;
        use windows_sys::Win32::Storage::FileSystem::*;
        let descriptor = local_descriptor("00112233445566778899aabbccddeeff");
        let _server = local_pipe(&descriptor.endpoint, PIPE_ACCESS_DUPLEX);
        let path: Vec<u16> = descriptor.endpoint.encode_utf16().chain([0]).collect();
        let client_raw = unsafe { CreateFileW(path.as_ptr(), GENERIC_READ | GENERIC_WRITE, 0, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut()) };
        assert_ne!(client_raw, INVALID_HANDLE_VALUE);
        let _client = unsafe { std::fs::File::from_raw_handle(client_raw) };
        let started = std::time::Instant::now();
        assert!(read_frame(&descriptor, 0).unwrap().is_null());
        assert!(started.elapsed() >= std::time::Duration::from_millis(240));
        assert!(started.elapsed() < std::time::Duration::from_millis(350));
    }
    #[cfg(windows)]
    #[test]
    fn incompatible_pipe_access_is_a_real_error_not_pending() {
        use windows_sys::Win32::Storage::FileSystem::PIPE_ACCESS_OUTBOUND;
        let descriptor = local_descriptor("00112233445566778899aabbccddeeff");
        let _server = local_pipe(&descriptor.endpoint, PIPE_ACCESS_OUTBOUND);
        let started = std::time::Instant::now();
        let error = read_frame(&descriptor, 0).unwrap_err();
        assert!(error.contains("Win32 5"), "{error}");
        assert!(started.elapsed() < std::time::Duration::from_millis(200));
    }
}
