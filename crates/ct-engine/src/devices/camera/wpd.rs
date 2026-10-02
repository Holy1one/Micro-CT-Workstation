//! Independently implemented Microsoft WPD/MTP extension transport.
//! No PortableDeviceLib code, vendor DLL, driver replacement or bridge process.
//! All COM interfaces live exclusively on the camera actor thread.

use super::ptp::*;
use super::types::*;

/// The production WPD reader and offline responses share this state machine.
/// A full bounded read is not EOF when the driver reports an unknown total.
struct ReadTransfer {
    max: usize,
    total: Option<usize>,
    chunk_size: usize,
    data: Vec<u8>,
    complete: bool,
}
impl ReadTransfer {
    fn new(total: u64, max: usize, suggested: usize) -> Result<Self, CameraError> {
        if max == 0 || max > MAX_OBJECT_BYTES {
            return Err(invalid("invalid bounded PTP read size"));
        }
        let total = if matches!(total, 0xffffffff | u64::MAX) {
            None
        } else {
            if total > max as u64 {
                return Err(invalid("WPD transfer exceeds operation size bound"));
            }
            Some(total as usize)
        };
        Ok(Self {
            max,
            total,
            chunk_size: suggested.clamp(1024, 64 * 1024),
            data: Vec::with_capacity(total.unwrap_or(0)),
            complete: total == Some(0),
        })
    }
    fn next_request(&self, context: &CallContext) -> Result<Option<usize>, CameraError> {
        context.check()?;
        if self.complete {
            return Ok(None);
        }
        let remaining = self.total.unwrap_or(self.max) - self.data.len();
        // The one-byte probe is never appended. Nonzero data proves oversize;
        // only a zero-byte response proves EOF exactly at the bounded length.
        Ok(Some(if remaining == 0 {
            1
        } else {
            remaining.min(self.chunk_size)
        }))
    }
    fn accept_chunk(
        &mut self,
        requested: usize,
        read: usize,
        bytes: &[u8],
        context: &CallContext,
    ) -> Result<(), CameraError> {
        if self.next_request(context)? != Some(requested)
            || read > requested
            || read > bytes.len()
            || bytes.len() > requested
        {
            return Err(invalid("WPD transfer length/read count mismatch"));
        }
        if self.total.is_none() && self.data.len() == self.max {
            if read != 0 {
                return Err(invalid("unknown-size WPD transfer exceeded cap"));
            }
            self.complete = true;
            return Ok(());
        }
        if self.total.is_some() && read == 0 {
            return Err(invalid("truncated WPD transfer"));
        }
        self.data.extend_from_slice(&bytes[..read]);
        self.complete = self.total.is_some_and(|total| self.data.len() == total)
            || self.total.is_none() && read < requested;
        Ok(())
    }
    fn finish(self, response_code: u16, context: &CallContext) -> Result<Vec<u8>, CameraError> {
        context.check()?;
        if !self.complete {
            return Err(invalid("WPD transfer ended without length/EOF evidence"));
        }
        if response_code != OK {
            return Err(CameraError::Command(format!(
                "PTP end-transfer response 0x{response_code:04x}"
            )));
        }
        Ok(self.data)
    }
}

#[cfg(windows)]
mod windows_transport {
    use super::*;
    use windows::core::{w, PCWSTR, PWSTR};
    use windows::Win32::Devices::PortableDevices::*;
    use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, PROPERTYKEY};
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Com::*;

    fn com_error(error: windows::core::Error) -> CameraError {
        CameraError::Unavailable(format!("Windows WPD: {error}"))
    }
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }
    unsafe fn values() -> Result<IPortableDeviceValues, CameraError> {
        unsafe {
            CoCreateInstance(&PortableDeviceValues, None, CLSCTX_INPROC_SERVER).map_err(com_error)
        }
    }
    unsafe fn command(code: &PROPERTYKEY) -> Result<IPortableDeviceValues, CameraError> {
        unsafe {
            let v = values()?;
            v.SetGuidValue(&WPD_PROPERTY_COMMON_COMMAND_CATEGORY, &code.fmtid)
                .map_err(com_error)?;
            v.SetUnsignedIntegerValue(&WPD_PROPERTY_COMMON_COMMAND_ID, code.pid)
                .map_err(com_error)?;
            Ok(v)
        }
    }
    unsafe fn allocated_string(
        v: &IPortableDeviceValues,
        key: &PROPERTYKEY,
    ) -> Result<String, CameraError> {
        unsafe {
            let raw = v.GetStringValue(key).map_err(com_error)?;
            let result = raw
                .to_string()
                .map_err(|_| invalid("WPD string is invalid UTF-16"));
            CoTaskMemFree(Some(raw.0.cast()));
            result
        }
    }
    unsafe fn buffer(v: &IPortableDeviceValues, max: usize) -> Result<Vec<u8>, CameraError> {
        unsafe {
            let mut pointer = std::ptr::null_mut();
            let mut size = 0;
            v.GetBufferValue(&WPD_PROPERTY_MTP_EXT_TRANSFER_DATA, &mut pointer, &mut size)
                .map_err(com_error)?;
            let result = if size as usize > max || (pointer.is_null() && size > 0) {
                Err(invalid("WPD buffer violates transfer bound"))
            } else if size == 0 {
                Ok(vec![])
            } else {
                Ok(std::slice::from_raw_parts(pointer, size as usize).to_vec())
            };
            CoTaskMemFree(Some(pointer.cast()));
            result
        }
    }
    pub(crate) struct WpdTransport {
        device: Option<IPortableDevice>,
    }
    impl WpdTransport {
        pub fn new() -> Result<Self, CameraError> {
            unsafe {
                CoInitializeEx(None, COINIT_MULTITHREADED)
                    .ok()
                    .map_err(com_error)?;
            }
            Ok(Self { device: None })
        }
        fn send(
            &self,
            request: &IPortableDeviceValues,
            context: &CallContext,
        ) -> Result<IPortableDeviceValues, CameraError> {
            context.check()?;
            let d = self
                .device
                .as_ref()
                .ok_or_else(|| CameraError::Unavailable("WPD device is not open".into()))?;
            // A Windows driver may block this synchronous call. The caller's
            // watchdog invalidates the epoch and isolates this session without
            // waiting for COM. The post-call check forbids all late side effects.
            let result = unsafe { d.SendCommand(0, request).map_err(com_error)? };
            context.check()?;
            unsafe {
                result
                    .GetErrorValue(&WPD_PROPERTY_COMMON_HRESULT)
                    .map_err(com_error)?
                    .ok()
                    .map_err(com_error)?;
            }
            Ok(result)
        }
        unsafe fn initial(
            &self,
            opcode: u16,
            params: &[u32],
            code: &PROPERTYKEY,
        ) -> Result<IPortableDeviceValues, CameraError> {
            unsafe {
                if params.len() > 5 {
                    return Err(invalid("PTP operation parameter count exceeds 5"));
                }
                let v = command(code)?;
                v.SetUnsignedIntegerValue(&WPD_PROPERTY_MTP_EXT_OPERATION_CODE, opcode as u32)
                    .map_err(com_error)?;
                let collection: IPortableDevicePropVariantCollection = CoCreateInstance(
                    &PortableDevicePropVariantCollection,
                    None,
                    CLSCTX_INPROC_SERVER,
                )
                .map_err(com_error)?;
                for param in params {
                    collection
                        .Add(&PROPVARIANT::from(*param))
                        .map_err(com_error)?;
                }
                v.SetIPortableDevicePropVariantCollectionValue(
                    &WPD_PROPERTY_MTP_EXT_OPERATION_PARAMS,
                    &collection,
                )
                .map_err(com_error)?;
                Ok(v)
            }
        }
        fn end(&self, transfer: &str, context: &CallContext) -> Result<u16, CameraError> {
            unsafe {
                let v = command(&WPD_COMMAND_MTP_EXT_END_DATA_TRANSFER)?;
                let context_wide = wide(transfer);
                v.SetStringValue(
                    &WPD_PROPERTY_MTP_EXT_TRANSFER_CONTEXT,
                    PCWSTR(context_wide.as_ptr()),
                )
                .map_err(com_error)?;
                let result = self.send(&v, context)?;
                u16::try_from(
                    result
                        .GetUnsignedIntegerValue(&WPD_PROPERTY_MTP_EXT_RESPONSE_CODE)
                        .map_err(com_error)?,
                )
                .map_err(|_| invalid("WPD response code exceeds PTP width"))
            }
        }
    }
    impl CameraTransport for WpdTransport {
        fn discover(&mut self, context: &CallContext) -> Result<Vec<CameraIdentity>, CameraError> {
            context.check()?;
            unsafe {
                let manager: IPortableDeviceManager =
                    CoCreateInstance(&PortableDeviceManager, None, CLSCTX_INPROC_SERVER)
                        .map_err(com_error)?;
                manager.RefreshDeviceList().map_err(com_error)?;
                let mut count = 0;
                manager
                    .GetDevices(std::ptr::null_mut(), &mut count)
                    .map_err(com_error)?;
                if count > 128 {
                    return Err(invalid("WPD discovery count exceeds limit"));
                }
                if count == 0 {
                    return Ok(vec![]);
                }
                let mut ids = vec![PWSTR::null(); count as usize];
                let result = manager.GetDevices(ids.as_mut_ptr(), &mut count);
                if count as usize > ids.len() {
                    for id in ids {
                        CoTaskMemFree(Some(id.0.cast()));
                    }
                    return Err(invalid("WPD device list changed beyond allocated count"));
                }
                let mut found = vec![];
                for id in ids {
                    if !id.is_null() {
                        if result.is_ok() {
                            let id_string = id
                                .to_string()
                                .map_err(|_| invalid("WPD device ID is invalid UTF-16"));
                            let mut size = 0;
                            let _ = manager.GetDeviceFriendlyName(
                                PCWSTR(id.0),
                                PWSTR::null(),
                                &mut size,
                            );
                            let mut name = vec![0u16; size.min(4096) as usize];
                            let friendly = if size > 0
                                && size <= 4096
                                && manager
                                    .GetDeviceFriendlyName(
                                        PCWSTR(id.0),
                                        PWSTR(name.as_mut_ptr()),
                                        &mut size,
                                    )
                                    .is_ok()
                            {
                                String::from_utf16_lossy(
                                    &name
                                        [..name.iter().position(|n| *n == 0).unwrap_or(name.len())],
                                )
                            } else {
                                "Portable device (identity unverified)".into()
                            };
                            if let Ok(id_string) = id_string {
                                found.push(CameraIdentity {
                                    device_id: id_string,
                                    manufacturer: String::new(),
                                    model: friendly,
                                    serial: None,
                                    backend: "windows_wpd_unverified".into(),
                                });
                            }
                        }
                        CoTaskMemFree(Some(id.0.cast()));
                    }
                }
                result.map_err(com_error)?;
                context.check()?;
                Ok(found)
            }
        }
        fn open(&mut self, id: &str, context: &CallContext) -> Result<(), CameraError> {
            context.check()?;
            if self.device.is_some() {
                return Err(CameraError::Busy("WPD device already open".into()));
            }
            if id.is_empty() || id.contains('\0') || id.len() > 4096 {
                return Err(invalid("invalid selected WPD device ID"));
            }
            unsafe {
                let device: IPortableDevice =
                    CoCreateInstance(&PortableDevice, None, CLSCTX_INPROC_SERVER)
                        .map_err(com_error)?;
                let client = values()?;
                client
                    .SetStringValue(&WPD_CLIENT_NAME, w!("MicroCT native camera"))
                    .map_err(com_error)?;
                client
                    .SetUnsignedIntegerValue(&WPD_CLIENT_MAJOR_VERSION, 1)
                    .map_err(com_error)?;
                client
                    .SetUnsignedIntegerValue(&WPD_CLIENT_MINOR_VERSION, 0)
                    .map_err(com_error)?;
                client
                    .SetUnsignedIntegerValue(
                        &WPD_CLIENT_DESIRED_ACCESS,
                        GENERIC_READ.0 | GENERIC_WRITE.0,
                    )
                    .map_err(com_error)?;
                client
                    .SetUnsignedIntegerValue(&WPD_CLIENT_SHARE_MODE, 0)
                    .map_err(com_error)?;
                let device_id = wide(id);
                device
                    .Open(PCWSTR(device_id.as_ptr()), &client)
                    .map_err(com_error)?;
                self.device = Some(device);
                context.check()
            }
        }
        fn close(&mut self) -> Result<(), CameraError> {
            if let Some(device) = self.device.take() {
                unsafe {
                    device.Close().map_err(com_error)?;
                }
            }
            Ok(())
        }
        fn cancel(&mut self) -> Result<(), CameraError> {
            if let Some(device) = &self.device {
                unsafe {
                    device.Cancel().map_err(com_error)?;
                }
            }
            Ok(())
        }
        fn execute(
            &mut self,
            opcode: u16,
            params: &[u32],
            phase: DataPhase,
            context: &CallContext,
        ) -> Result<PtpResponse, CameraError> {
            context.check()?;
            unsafe {
                match phase {
                    DataPhase::None => {
                        let v = self.initial(
                            opcode,
                            params,
                            &WPD_COMMAND_MTP_EXT_EXECUTE_COMMAND_WITHOUT_DATA_PHASE,
                        )?;
                        let result = self.send(&v, context)?;
                        let code = u16::try_from(
                            result
                                .GetUnsignedIntegerValue(&WPD_PROPERTY_MTP_EXT_RESPONSE_CODE)
                                .map_err(com_error)?,
                        )
                        .map_err(|_| invalid("PTP response width"))?;
                        Ok(PtpResponse { code, data: vec![] })
                    }
                    DataPhase::Read { max } => {
                        if max == 0 || max > MAX_OBJECT_BYTES {
                            return Err(invalid("invalid bounded PTP read size"));
                        }
                        let v = self.initial(
                            opcode,
                            params,
                            &WPD_COMMAND_MTP_EXT_EXECUTE_COMMAND_WITH_DATA_TO_READ,
                        )?;
                        let initial = self.send(&v, context)?;
                        if let Ok(code) =
                            initial.GetUnsignedIntegerValue(&WPD_PROPERTY_MTP_EXT_RESPONSE_CODE)
                        {
                            if code != OK as u32 {
                                return Ok(PtpResponse {
                                    code: u16::try_from(code)
                                        .map_err(|_| invalid("PTP response width"))?,
                                    data: vec![],
                                });
                            }
                        }
                        let transfer =
                            allocated_string(&initial, &WPD_PROPERTY_MTP_EXT_TRANSFER_CONTEXT)?;
                        let total = initial
                            .GetUnsignedLargeIntegerValue(
                                &WPD_PROPERTY_MTP_EXT_TRANSFER_TOTAL_DATA_SIZE,
                            )
                            .map_err(com_error)?;
                        let suggested = initial
                            .GetUnsignedIntegerValue(
                                &WPD_PROPERTY_MTP_EXT_OPTIMAL_TRANSFER_BUFFER_SIZE,
                            )
                            .unwrap_or(64 * 1024) as usize;
                        let mut read_transfer = match ReadTransfer::new(total, max, suggested) {
                            Ok(value) => value,
                            Err(error) => {
                                let _ = self.end(&transfer, context);
                                return Err(error);
                            }
                        };
                        let transfer_wide = wide(&transfer);
                        while let Some(request_size) = read_transfer.next_request(context)? {
                            let request = command(&WPD_COMMAND_MTP_EXT_READ_DATA)?;
                            request
                                .SetStringValue(
                                    &WPD_PROPERTY_MTP_EXT_TRANSFER_CONTEXT,
                                    PCWSTR(transfer_wide.as_ptr()),
                                )
                                .map_err(com_error)?;
                            request
                                .SetUnsignedIntegerValue(
                                    &WPD_PROPERTY_MTP_EXT_TRANSFER_NUM_BYTES_TO_READ,
                                    request_size as u32,
                                )
                                .map_err(com_error)?;
                            request
                                .SetBufferValue(
                                    &WPD_PROPERTY_MTP_EXT_TRANSFER_DATA,
                                    &vec![0; request_size],
                                )
                                .map_err(com_error)?;
                            let result = self.send(&request, context)?;
                            let read = result
                                .GetUnsignedIntegerValue(
                                    &WPD_PROPERTY_MTP_EXT_TRANSFER_NUM_BYTES_READ,
                                )
                                .map_err(com_error)?
                                as usize;
                            let chunk = buffer(&result, request_size)?;
                            read_transfer.accept_chunk(request_size, read, &chunk, context)?;
                        }
                        let code = self.end(&transfer, context)?;
                        let data = read_transfer.finish(code, context)?;
                        Ok(PtpResponse { code, data })
                    }
                    DataPhase::Write(data) => {
                        if data.len() > 128 * 1024 {
                            return Err(invalid("PTP property write exceeds cap"));
                        }
                        let v = self.initial(
                            opcode,
                            params,
                            &WPD_COMMAND_MTP_EXT_EXECUTE_COMMAND_WITH_DATA_TO_WRITE,
                        )?;
                        v.SetUnsignedLargeIntegerValue(
                            &WPD_PROPERTY_MTP_EXT_TRANSFER_TOTAL_DATA_SIZE,
                            data.len() as u64,
                        )
                        .map_err(com_error)?;
                        let initial = self.send(&v, context)?;
                        if let Ok(code) =
                            initial.GetUnsignedIntegerValue(&WPD_PROPERTY_MTP_EXT_RESPONSE_CODE)
                        {
                            if code != OK as u32 {
                                return Ok(PtpResponse {
                                    code: u16::try_from(code)
                                        .map_err(|_| invalid("PTP response width"))?,
                                    data: vec![],
                                });
                            }
                        }
                        let transfer =
                            allocated_string(&initial, &WPD_PROPERTY_MTP_EXT_TRANSFER_CONTEXT)?;
                        let transfer_wide = wide(&transfer);
                        let write = command(&WPD_COMMAND_MTP_EXT_WRITE_DATA)?;
                        write
                            .SetStringValue(
                                &WPD_PROPERTY_MTP_EXT_TRANSFER_CONTEXT,
                                PCWSTR(transfer_wide.as_ptr()),
                            )
                            .map_err(com_error)?;
                        write
                            .SetUnsignedIntegerValue(
                                &WPD_PROPERTY_MTP_EXT_TRANSFER_NUM_BYTES_TO_WRITE,
                                data.len() as u32,
                            )
                            .map_err(com_error)?;
                        write
                            .SetBufferValue(&WPD_PROPERTY_MTP_EXT_TRANSFER_DATA, &data)
                            .map_err(com_error)?;
                        let result = self.send(&write, context)?;
                        if result
                            .GetUnsignedIntegerValue(
                                &WPD_PROPERTY_MTP_EXT_TRANSFER_NUM_BYTES_WRITTEN,
                            )
                            .map_err(com_error)? as usize
                            != data.len()
                        {
                            return Err(invalid("partial WPD property write"));
                        }
                        let code = self.end(&transfer, context)?;
                        context.check()?;
                        Ok(PtpResponse { code, data: vec![] })
                    }
                }
            }
        }
    }
    impl Drop for WpdTransport {
        fn drop(&mut self) {
            let _ = self.close();
            unsafe {
                CoUninitialize();
            }
        }
    }
}

pub(crate) fn create_transport() -> Result<Box<dyn CameraTransport>, CameraError> {
    #[cfg(windows)]
    {
        Ok(Box::new(windows_transport::WpdTransport::new()?))
    }
    #[cfg(not(windows))]
    {
        Err(CameraError::Unavailable(
            "native camera transport requires Windows WPD".into(),
        ))
    }
}

#[cfg(test)]
mod transfer_tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    };
    use std::time::{Duration, Instant};
    fn context() -> CallContext {
        CallContext {
            deadline: Instant::now() + Duration::from_secs(1),
            epoch: 1,
            generation: Arc::new(AtomicU64::new(1)),
        }
    }

    #[test]
    fn unknown_total_exact_object_size_requires_zero_probe_and_successful_end() {
        for sentinel in [0xffffffff, u64::MAX] {
            let context = context();
            let mut read = ReadTransfer::new(sentinel, 1024, 1024).unwrap();
            let request = read.next_request(&context).unwrap().unwrap();
            assert_eq!(request, 1024);
            read.accept_chunk(request, 1024, &[7; 1024], &context)
                .unwrap();
            assert_eq!(read.next_request(&context).unwrap(), Some(1));
            read.accept_chunk(1, 0, &[0], &context).unwrap();
            assert_eq!(read.next_request(&context).unwrap(), None);
            assert_eq!(read.finish(OK, &context).unwrap(), vec![7; 1024]);
        }
    }
    #[test]
    fn unknown_total_nonzero_probe_is_oversize_and_missing_probe_is_not_complete() {
        let context = context();
        let mut read = ReadTransfer::new(0xffffffff, 1024, 1024).unwrap();
        read.accept_chunk(1024, 1024, &[7; 1024], &context).unwrap();
        assert!(read.accept_chunk(1, 1, &[9], &context).is_err());
        assert!(read.finish(OK, &context).is_err());
    }
    #[test]
    fn short_unknown_read_known_truncation_and_end_failure_are_distinct() {
        let context = context();
        let mut short = ReadTransfer::new(0xffffffff, 1024, 1024).unwrap();
        short.accept_chunk(1024, 8, &[1; 1024], &context).unwrap();
        assert_eq!(short.finish(OK, &context).unwrap(), vec![1; 8]);
        let mut known = ReadTransfer::new(1024, 1024, 1024).unwrap();
        assert!(known.accept_chunk(1024, 0, &[], &context).is_err());
        let mut complete = ReadTransfer::new(8, 1024, 1024).unwrap();
        complete.accept_chunk(8, 8, &[3; 8], &context).unwrap();
        assert!(complete.finish(0x2002, &context).is_err());
    }
    #[test]
    fn eof_probe_and_end_keep_the_shared_deadline_and_cancel_generation() {
        let context = context();
        let mut read = ReadTransfer::new(0xffffffff, 1, 1024).unwrap();
        read.accept_chunk(1, 1, &[7], &context).unwrap();
        context.generation.store(2, Ordering::Release);
        assert!(matches!(
            read.next_request(&context),
            Err(CameraError::Cancelled)
        ));
        assert!(matches!(
            read.accept_chunk(1, 0, &[], &context),
            Err(CameraError::Cancelled)
        ));
        let expired = CallContext {
            deadline: Instant::now() - Duration::from_millis(1),
            epoch: 1,
            generation: Arc::new(AtomicU64::new(1)),
        };
        assert!(matches!(
            ReadTransfer::new(0, 1, 1024).unwrap().finish(OK, &expired),
            Err(CameraError::Timeout(_))
        ));
    }
}
