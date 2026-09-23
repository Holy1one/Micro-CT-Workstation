//! Turntable device module.
//!
//! `nano` implements the active RTS9060 serial contract. The module name uses
//! the device role rather than the current board so another controller can be
//! added later without changing scan-domain terminology.

mod nano;

#[cfg(test)]
pub(crate) use nano::LineTransport;

pub use nano::{
    MoveTicket, NanoAdapter, NanoConnectionState, NanoError, NanoHealth,
    NanoIdentity, NanoStatus, BAUD_RATE, EXPECTED_DEVICE, EXPECTED_PROTOCOL,
};
