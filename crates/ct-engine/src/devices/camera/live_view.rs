//! Validated JPEG frames in a single replaceable slot; binary data never enters
//! a JSONL snapshot. A separate pipe writer cannot block camera control or Stop.

use super::ptp::invalid;
use super::types::CameraError;
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const LIVE_VIEW_PIPE_VERSION: u16 = 1;
pub const LIVE_VIEW_HEADER_BYTES: usize = 64;
pub const MAX_LIVE_VIEW_BYTES: usize = 4 * 1024 * 1024;
pub const LIVE_VIEW_MAGIC: &[u8; 8] = b"MCTLV001";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveViewPipeDescriptor {
    pub endpoint: String,
    pub session_id: String,
    pub version: u16,
    pub max_frame_bytes: usize,
}
#[derive(Clone, Debug)]
pub struct LiveViewFrame {
    pub session_id: String,
    pub sequence: u64,
    pub width: u32,
    pub height: u32,
    pub captured_at_ms: u64,
    pub jpeg: Vec<u8>,
}

pub(crate) fn random_session() -> Result<String, CameraError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|e| CameraError::Unavailable(format!("secure session randomness: {e}")))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn session_bytes(session: &str) -> Result<[u8; 16], CameraError> {
    if session.len() != 32 || !session.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(invalid("invalid frame session ID"));
    }
    let mut bytes = [0; 16];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&session[i * 2..i * 2 + 2], 16)
            .map_err(|_| invalid("invalid session ID"))?;
    }
    Ok(bytes)
}
pub fn validate_jpeg(jpeg: &[u8], expected: Option<(u32, u32)>) -> Result<(u32, u32), CameraError> {
    if !(4..=MAX_LIVE_VIEW_BYTES).contains(&jpeg.len())
        || !jpeg.starts_with(&[0xff, 0xd8])
        || !jpeg.ends_with(&[0xff, 0xd9])
    {
        return Err(invalid("truncated or oversized Live View JPEG"));
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(jpeg), image::ImageFormat::Jpeg);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(3072);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|e| CameraError::Safety(format!("Live View JPEG decode: {e}")))?;
    let dimensions = (decoded.width(), decoded.height());
    if dimensions.0 == 0 || dimensions.1 == 0 || expected.is_some_and(|pair| pair != dimensions) {
        return Err(invalid("Live View JPEG/header dimensions differ"));
    }
    Ok(dimensions)
}
pub(crate) fn d7100_frame(
    bytes: &[u8],
    session: &str,
    sequence: u64,
) -> Result<LiveViewFrame, CameraError> {
    if bytes.len() <= 384 || bytes.len() > MAX_LIVE_VIEW_BYTES + 384 {
        return Err(invalid(
            "D7100 Live View requires bounded 384-byte header plus JPEG",
        ));
    }
    let width = u16::from_le_bytes([bytes[8], bytes[9]]) as u32;
    let height = u16::from_le_bytes([bytes[10], bytes[11]]) as u32;
    validate_jpeg(&bytes[384..], Some((width, height)))?;
    Ok(LiveViewFrame {
        session_id: session.into(),
        sequence,
        width,
        height,
        captured_at_ms: chrono::Utc::now().timestamp_millis().max(0) as u64,
        jpeg: bytes[384..].to_vec(),
    })
}
pub fn encode_live_view_frame(frame: &LiveViewFrame) -> Result<Vec<u8>, CameraError> {
    validate_jpeg(&frame.jpeg, Some((frame.width, frame.height)))?;
    let mut bytes = Vec::with_capacity(LIVE_VIEW_HEADER_BYTES + frame.jpeg.len());
    bytes.extend(LIVE_VIEW_MAGIC);
    bytes.extend(LIVE_VIEW_PIPE_VERSION.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend((LIVE_VIEW_HEADER_BYTES as u32).to_le_bytes());
    bytes.extend(session_bytes(&frame.session_id)?);
    bytes.extend(frame.sequence.to_le_bytes());
    bytes.extend(frame.width.to_le_bytes());
    bytes.extend(frame.height.to_le_bytes());
    bytes.extend((frame.jpeg.len() as u32).to_le_bytes());
    bytes.extend(0u32.to_le_bytes());
    bytes.extend(frame.captured_at_ms.to_le_bytes());
    bytes.extend(&frame.jpeg);
    Ok(bytes)
}
/// The desktop forwarder must pass its engine-issued expected session and last
/// accepted sequence. Truncated, duplicate, old, oversized and foreign frames fail.
pub fn decode_live_view_frame(
    bytes: &[u8],
    session: &str,
    after: u64,
) -> Result<LiveViewFrame, CameraError> {
    if bytes.len() < LIVE_VIEW_HEADER_BYTES || &bytes[..8] != LIVE_VIEW_MAGIC {
        return Err(invalid("invalid Live View envelope"));
    }
    let u16_at = |at| u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap());
    let u32_at = |at| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let u64_at = |at| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    if u16_at(8) != LIVE_VIEW_PIPE_VERSION
        || u16_at(10) != 1
        || u32_at(12) != LIVE_VIEW_HEADER_BYTES as u32
        || bytes[16..32] != session_bytes(session)?
        || u32_at(52) != 0
    {
        return Err(invalid(
            "Live View envelope version/codec/session/reserved mismatch",
        ));
    }
    let sequence = u64_at(32);
    let len = u32_at(48) as usize;
    if sequence == 0
        || sequence <= after
        || len > MAX_LIVE_VIEW_BYTES
        || len + LIVE_VIEW_HEADER_BYTES != bytes.len()
    {
        return Err(invalid("stale, duplicate, truncated or oversized frame"));
    }
    let width = u32_at(40);
    let height = u32_at(44);
    validate_jpeg(&bytes[LIVE_VIEW_HEADER_BYTES..], Some((width, height)))?;
    Ok(LiveViewFrame {
        session_id: session.into(),
        sequence,
        width,
        height,
        captured_at_ms: u64_at(56),
        jpeg: bytes[LIVE_VIEW_HEADER_BYTES..].to_vec(),
    })
}

#[derive(Default)]
struct FrameSlot {
    frame: Option<LiveViewFrame>,
    received: Option<Instant>,
    descriptor: Option<LiveViewPipeDescriptor>,
}
#[derive(Clone, Default)]
pub(crate) struct LatestFrameSlot(Arc<Mutex<FrameSlot>>);
impl LatestFrameSlot {
    pub fn publish(&self, frame: LiveViewFrame) {
        let mut s = self.0.lock().unwrap();
        if s.descriptor
            .as_ref()
            .is_some_and(|d| d.session_id == frame.session_id)
        {
            s.frame = Some(frame);
            s.received = Some(Instant::now());
        }
    }
    pub fn revoke(&self) {
        let mut s = self.0.lock().unwrap();
        s.frame = None;
        s.received = None;
        s.descriptor = None;
    }
    pub fn install(&self, descriptor: LiveViewPipeDescriptor) {
        let mut s = self.0.lock().unwrap();
        s.frame = None;
        s.received = None;
        s.descriptor = Some(descriptor);
    }
    pub fn descriptor(&self) -> Option<LiveViewPipeDescriptor> {
        self.0.lock().unwrap().descriptor.clone()
    }
    pub fn is_fresh(&self) -> bool {
        self.0
            .lock()
            .unwrap()
            .received
            .is_some_and(|t| t.elapsed() <= Duration::from_secs(1))
    }
    pub fn latest(&self) -> Option<LiveViewFrame> {
        let s = self.0.lock().unwrap();
        if s.received
            .is_some_and(|t| t.elapsed() <= Duration::from_secs(1))
        {
            s.frame.clone()
        } else {
            None
        }
    }
}

#[cfg(windows)]
pub(crate) fn start_frame_pipe(
    slot: LatestFrameSlot,
    session: &str,
) -> Result<LiveViewPipeDescriptor, CameraError> {
    let descriptor = LiveViewPipeDescriptor {
        endpoint: format!(r"\\.\pipe\microct-lv-{}", random_session()?),
        session_id: session.into(),
        version: LIVE_VIEW_PIPE_VERSION,
        max_frame_bytes: MAX_LIVE_VIEW_BYTES,
    };
    // Prepare the user-only ACL and first pipe synchronously before publishing.
    let security = pipe_windows::user_security_descriptor()?;
    let pipe = pipe_windows::Pipe::create(&descriptor.endpoint, &security)?;
    slot.install(descriptor.clone());
    let endpoint = descriptor.endpoint.clone();
    std::thread::Builder::new()
        .name("camera-lv-pipe".into())
        .spawn(move || {
            let mut pipe = Some(pipe);
            while slot.descriptor().is_some_and(|d| d.endpoint == endpoint) {
                if let Some(p) = pipe.take() {
                    if p.connect(Duration::from_millis(50)).is_ok() {
                        if let Some(frame) = slot.latest() {
                            if let Ok(bytes) = encode_live_view_frame(&frame) {
                                let _ = p.write(&bytes, Duration::from_millis(50));
                            }
                        }
                    }
                    drop(p);
                }
                if slot.descriptor().is_none_or(|d| d.endpoint != endpoint) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
                pipe = pipe_windows::Pipe::create(&endpoint, &security).ok();
            }
        })
        .map_err(|e| CameraError::Io(e.to_string()))?;
    Ok(descriptor)
}
#[cfg(not(windows))]
pub(crate) fn start_frame_pipe(
    slot: LatestFrameSlot,
    session: &str,
) -> Result<LiveViewPipeDescriptor, CameraError> {
    // Non-Windows builds can exercise parsers and the same latest-only slot;
    // production Live View has no fallback transport.
    let _ = (slot, session);
    Err(CameraError::Unavailable(
        "Windows named pipe transport required".into(),
    ))
}

#[cfg(windows)]
mod pipe_windows {
    use super::*;
    use windows::core::{HRESULT, PCWSTR, PWSTR};
    use windows::Win32::Foundation::*;
    use windows::Win32::Security::Authorization::*;
    use windows::Win32::Security::*;
    use windows::Win32::Storage::FileSystem::*;
    use windows::Win32::System::Pipes::*;
    use windows::Win32::System::Threading::*;
    use windows::Win32::System::IO::*;

    fn err(e: windows::core::Error) -> CameraError {
        CameraError::Io(format!("Live View pipe: {e}"))
    }
    pub(super) fn user_security_descriptor() -> Result<Vec<u8>, CameraError> {
        unsafe {
            let mut token = HANDLE::default();
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).map_err(err)?;
            let mut size = 0;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut size);
            if size == 0 {
                let _ = CloseHandle(token);
                return Err(invalid("cannot determine current user SID"));
            }
            // Vec<u64> ensures TOKEN_USER alignment.
            let mut buffer = vec![0u64; (size as usize + 7) / 8];
            let result = GetTokenInformation(
                token,
                TokenUser,
                Some(buffer.as_mut_ptr().cast()),
                size,
                &mut size,
            );
            let _ = CloseHandle(token);
            result.map_err(err)?;
            let user = &*(buffer.as_ptr().cast::<TOKEN_USER>());
            let mut sid = PWSTR::null();
            ConvertSidToStringSidW(user.User.Sid, &mut sid).map_err(err)?;
            let sid_string = sid
                .to_string()
                .map_err(|_| invalid("invalid current-user SID"));
            let _ = LocalFree(Some(HLOCAL(sid.0.cast())));
            let sid_string = sid_string?;
            let sddl: Vec<u16> = format!("D:P(A;;GRGW;;;{sid_string})")
                .encode_utf16()
                .chain([0])
                .collect();
            let mut raw = PSECURITY_DESCRIPTOR::default();
            let mut length = 0;
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut raw,
                Some(&mut length),
            )
            .map_err(err)?;
            let result = std::slice::from_raw_parts(raw.0.cast::<u8>(), length as usize).to_vec();
            let _ = LocalFree(Some(HLOCAL(raw.0)));
            Ok(result)
        }
    }
    pub(super) struct Pipe {
        handle: usize,
    }
    // HANDLE carries no COM object and is thread-agnostic; move its integer
    // representation instead of adding unsafe Send to camera COM types.
    impl Pipe {
        fn handle(&self) -> HANDLE {
            HANDLE(self.handle as *mut _)
        }
        pub fn create(endpoint: &str, security: &[u8]) -> Result<Self, CameraError> {
            let path: Vec<u16> = endpoint.encode_utf16().chain([0]).collect();
            let sa = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: security.as_ptr() as *mut _,
                bInheritHandle: false.into(),
            };
            let handle = unsafe {
                CreateNamedPipeW(
                    PCWSTR(path.as_ptr()),
                    PIPE_ACCESS_OUTBOUND | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                    1,
                    (MAX_LIVE_VIEW_BYTES + LIVE_VIEW_HEADER_BYTES) as u32,
                    0,
                    100,
                    Some(&sa),
                )
            };
            if handle == INVALID_HANDLE_VALUE {
                return Err(err(windows::core::Error::from_thread()));
            }
            Ok(Self {
                handle: handle.0 as usize,
            })
        }
        fn wait(&self, overlapped: &mut OVERLAPPED, timeout: Duration) -> Result<u32, CameraError> {
            unsafe {
                if WaitForSingleObject(overlapped.hEvent, timeout.as_millis() as u32)
                    != WAIT_OBJECT_0
                {
                    let _ = CancelIoEx(self.handle(), Some(overlapped));
                    // Drain cancellation before stack OVERLAPPED/buffers expire.
                    let mut transferred = 0;
                    let _ = GetOverlappedResult(self.handle(), overlapped, &mut transferred, true);
                    return Err(CameraError::Timeout(
                        "Live View consumer exceeded pipe deadline".into(),
                    ));
                }
                let mut transferred = 0;
                GetOverlappedResult(self.handle(), overlapped, &mut transferred, false)
                    .map_err(err)?;
                Ok(transferred)
            }
        }
        pub fn connect(&self, timeout: Duration) -> Result<(), CameraError> {
            unsafe {
                let event = CreateEventW(None, true, false, None).map_err(err)?;
                let mut overlapped = OVERLAPPED {
                    hEvent: event,
                    ..Default::default()
                };
                let result = match ConnectNamedPipe(self.handle(), Some(&mut overlapped)) {
                    Ok(()) => Ok(()),
                    Err(e) if e.code() == HRESULT::from_win32(ERROR_PIPE_CONNECTED.0) => Ok(()),
                    Err(e) if e.code() == HRESULT::from_win32(ERROR_IO_PENDING.0) => {
                        self.wait(&mut overlapped, timeout).map(|_| ())
                    }
                    Err(e) => Err(err(e)),
                };
                let _ = CloseHandle(event);
                result
            }
        }
        pub fn write(&self, bytes: &[u8], timeout: Duration) -> Result<(), CameraError> {
            unsafe {
                let event = CreateEventW(None, true, false, None).map_err(err)?;
                let mut overlapped = OVERLAPPED {
                    hEvent: event,
                    ..Default::default()
                };
                let mut written = 0;
                let result = match WriteFile(
                    self.handle(),
                    Some(bytes),
                    Some(&mut written),
                    Some(&mut overlapped),
                ) {
                    Ok(()) => Ok(written),
                    Err(e) if e.code() == HRESULT::from_win32(ERROR_IO_PENDING.0) => {
                        self.wait(&mut overlapped, timeout)
                    }
                    Err(e) => Err(err(e)),
                }
                .and_then(|n| {
                    if n as usize == bytes.len() {
                        Ok(())
                    } else {
                        Err(invalid("partial Live View pipe write"))
                    }
                });
                let _ = CloseHandle(event);
                result
            }
        }
    }
    impl Drop for Pipe {
        fn drop(&mut self) {
            unsafe {
                let _ = CancelIoEx(self.handle(), None);
                let _ = DisconnectNamedPipe(self.handle());
                let _ = CloseHandle(self.handle());
            }
        }
    }
}
