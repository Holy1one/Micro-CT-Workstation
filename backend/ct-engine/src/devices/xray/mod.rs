//! X-ray source device module.
//!
//! `moxtek` is the current USB/serial implementation. All callers must treat
//! readback as measured state and setpoints only as requested state.

mod moxtek;
mod worker;
mod transport;

pub use moxtek::{
    XrayError, XrayHealth, MAX_CURRENT_UA, MAX_SETPOINT_POWER_W,
    MAX_VOLTAGE_KV, MIN_VOLTAGE_KV,
};
pub use worker::MoxtekAdapter;
pub(crate) use moxtek::nominal_voltage;
#[cfg(test)]
pub(crate) use moxtek::tests::{test_worker, TestXrayProbe};
