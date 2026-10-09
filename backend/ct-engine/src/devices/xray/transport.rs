//! Bounded packet reception. Read slices retry the same pending reply; commands
//! are never replayed (the Moxtek protocol has no transaction identifier).
use serialport::{ClearBuffer, SerialPort};
use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

pub(super) const IO_TIMEOUT: Duration = Duration::from_secs(1);
const READ_SLICE: Duration = Duration::from_millis(50);

pub(super) trait BinaryTransport: Send {
    fn clear_input(&mut self) -> io::Result<()>;
    fn write_all_bytes(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn write_cancellable(&mut self, bytes: &[u8], _timeout: Duration, cancelled: &dyn Fn() -> bool) -> io::Result<()> {
        if cancelled() { return Err(io::ErrorKind::Interrupted.into()); }
        self.write_all_bytes(bytes)
    }
    fn read_exact_bytes(&mut self, bytes: &mut [u8], timeout: Duration) -> io::Result<()>;
    fn read_cancellable(&mut self, bytes: &mut [u8], timeout: Duration, cancelled: &dyn Fn() -> bool) -> io::Result<()> {
        if cancelled() { return Err(io::ErrorKind::Interrupted.into()); }
        self.read_exact_bytes(bytes, timeout)
    }
}

pub(super) struct SerialBinaryTransport {
    pub(super) port: Box<dyn SerialPort>,
}

impl BinaryTransport for SerialBinaryTransport {
    fn clear_input(&mut self) -> io::Result<()> {
        self.port.clear(ClearBuffer::Input)
            .map_err(|error| io::Error::other(error.to_string()))
    }

    fn write_all_bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.write_cancellable(bytes, IO_TIMEOUT, &|| false)
    }

    fn write_cancellable(&mut self, bytes: &[u8], timeout: Duration, cancelled: &dyn Fn() -> bool) -> io::Result<()> {
        let deadline = Instant::now() + timeout.min(IO_TIMEOUT);
        let mut written = 0;
        while written < bytes.len() {
            if cancelled() { return Err(io::ErrorKind::Interrupted.into()); }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(io::ErrorKind::TimedOut,
                    format!("Moxtek packet write timed out after {written}/{} bytes", bytes.len())));
            }
            self.port.set_timeout(remaining).map_err(|error| io::Error::other(error.to_string()))?;
            match self.port.write(&bytes[written..]) {
                Ok(0) => return Err(io::Error::new(io::ErrorKind::WriteZero, "Moxtek packet write made no progress")),
                Ok(count) => written += count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn read_exact_bytes(&mut self, bytes: &mut [u8], timeout: Duration) -> io::Result<()> {
        self.read_cancellable(bytes, timeout, &|| false)
    }

    fn read_cancellable(&mut self, bytes: &mut [u8], timeout: Duration, cancelled: &dyn Fn() -> bool) -> io::Result<()> {
        receive_packet(bytes, timeout, cancelled, |buffer, slice| {
            self.port.set_timeout(slice).map_err(|error| io::Error::other(error.to_string()))?;
            self.port.read(buffer)
        })
    }
}

/// A partial packet remains in place across OS read timeouts. A zero-byte read
/// is also a timeout on Windows VCP, not proof that the device disconnected.
/// Actual driver errors and exhausted budgets remain failures.
pub(super) fn receive_packet(
    bytes: &mut [u8], timeout: Duration, cancelled: &dyn Fn() -> bool,
    mut read: impl FnMut(&mut [u8], Duration) -> io::Result<usize>,
) -> io::Result<()> {
    let started = Instant::now();
    let deadline = started + timeout;
    let mut received = 0;
    while received < bytes.len() {
        if cancelled() { return Err(io::ErrorKind::Interrupted.into()); }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, format!(
                "Moxtek packet read timed out: received={received}/{}, elapsed={} ms, budget={} ms",
                bytes.len(), started.elapsed().as_millis(), timeout.as_millis())));
        }
        match read(&mut bytes[received..], remaining.min(READ_SLICE)) {
            Ok(0) => std::thread::sleep(remaining.min(Duration::from_millis(1))),
            Ok(count) if count <= bytes.len() - received => received += count,
            Ok(_) => return Err(io::Error::new(io::ErrorKind::InvalidData, "read returned an invalid byte count")),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock =>
                std::thread::sleep(remaining.min(Duration::from_millis(1))),
            Err(error) if matches!(error.kind(), io::ErrorKind::Interrupted | io::ErrorKind::TimedOut) => {},
            Err(error) => return Err(io::Error::new(error.kind(), format!(
                "Moxtek packet read failed: received={received}/{}, elapsed={} ms: {error}",
                bytes.len(), started.elapsed().as_millis()))),
        }
    }
    if cancelled() { return Err(io::ErrorKind::Interrupted.into()); }
    if Instant::now() > deadline {
        return Err(io::Error::new(io::ErrorKind::TimedOut, format!(
            "Moxtek packet arrived after its read deadline: received={received}/{}, elapsed={} ms, budget={} ms",
            bytes.len(), started.elapsed().as_millis(), timeout.as_millis())));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn delayed_fragmented_reply_survives_read_timeouts_without_losing_bytes() {
        let mut packet = [0; 5];
        let mut step = 0;
        receive_packet(&mut packet, Duration::from_millis(200), &|| false, |buffer, slice| {
            assert!(slice <= READ_SLICE);
            step += 1;
            match step {
                1 => { buffer[..2].copy_from_slice(&[0x1B, 0x43]); Ok(2) },
                2 => Err(io::ErrorKind::TimedOut.into()),
                3 => Ok(0),
                4 => Err(io::ErrorKind::WouldBlock.into()),
                _ => { buffer.copy_from_slice(&[2, 8, 9]); Ok(3) },
            }
        }).unwrap();
        assert_eq!(packet, [0x1B, 0x43, 2, 8, 9]);
    }

    #[test]
    fn partial_reply_times_out_with_progress_and_bounded_elapsed_time() {
        let start = Instant::now();
        let mut first = true;
        let error = receive_packet(&mut [0; 8], Duration::from_millis(100), &|| false, |buffer, slice| {
            if first { first = false; buffer[0] = 0x1B; return Ok(1); }
            std::thread::sleep(slice);
            Err(io::ErrorKind::TimedOut.into())
        }).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(error.to_string().contains("received=1/8"));
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn permanent_no_response_is_bounded_and_real_disconnection_is_immediate() {
        let error = receive_packet(&mut [0; 8], Duration::from_millis(60), &|| false, |_, slice| {
            std::thread::sleep(slice); Ok(0)
        }).unwrap_err();
        assert!(error.to_string().contains("received=0/8"));
        let mut calls = 0;
        let error = receive_packet(&mut [0; 8], Duration::from_secs(5), &|| false, |_, _| {
            calls += 1; Err(io::ErrorKind::BrokenPipe.into())
        }).unwrap_err();
        assert_eq!(calls, 1);
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn cancellation_interrupts_a_five_second_read_after_one_slice() {
        let cancel = AtomicBool::new(false);
        let start = Instant::now();
        let error = receive_packet(&mut [0; 8], Duration::from_secs(5), &|| cancel.load(Ordering::SeqCst), |_, slice| {
            std::thread::sleep(slice); cancel.store(true, Ordering::SeqCst);
            Err(io::ErrorKind::TimedOut.into())
        }).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(start.elapsed() < Duration::from_millis(500));
    }
}
