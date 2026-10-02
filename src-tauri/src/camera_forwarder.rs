//! Thin, bounded Live View pipe reader. Only an engine-issued local endpoint is
//! accepted. The webview supplies a sequence number and never a file/pipe path;
//! JPEG validation is shared with the engine and no device state is owned here.

use ct_engine::devices::camera::{
    decode_live_view_frame, LiveViewPipeDescriptor, LIVE_VIEW_HEADER_BYTES, LIVE_VIEW_PIPE_VERSION,
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
    let bytes = read_pipe(&descriptor.endpoint)?;
    let frame = decode_live_view_frame(&bytes, &descriptor.session_id, after_sequence)
        .map_err(|error| error.to_string())?;
    Ok(
        json!({"sessionId":frame.session_id,"sequence":frame.sequence,"width":frame.width,"height":frame.height,"capturedAtMs":frame.captured_at_ms,"jpeg":frame.jpeg}),
    )
}

#[cfg(not(windows))]
fn read_pipe(_endpoint: &str) -> Result<Vec<u8>, String> {
    Err("Windows Live View pipe is unavailable".into())
}

#[cfg(windows)]
fn read_pipe(endpoint: &str) -> Result<Vec<u8>, String> {
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
    let path: Vec<u16> = endpoint.encode_utf16().chain([0]).collect();
    let deadline = Instant::now() + Duration::from_millis(250);
    unsafe {
        WaitNamedPipeW(path.as_ptr(), 50);
        let raw = CreateFileW(
            path.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED,
            std::ptr::null_mut(),
        );
        if raw == INVALID_HANDLE_VALUE {
            return Err(format!(
                "Live View endpoint unavailable: {}",
                std::io::Error::last_os_error()
            ));
        }
        let handle = OwnedHandle(raw);
        let event_raw = CreateEventW(std::ptr::null(), 1, 0, std::ptr::null());
        if event_raw.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let event = OwnedHandle(event_raw);
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
        Ok(bytes)
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
            version: 1,
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
}
