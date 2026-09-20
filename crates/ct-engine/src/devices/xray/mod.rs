//! X-ray source device module.
//!
//! `moxtek` is the current USB/serial implementation. All callers must treat
//! readback as measured state and setpoints only as requested state.

mod moxtek;

pub use moxtek::{
    MoxtekAdapter, XrayError, XrayHealth, MAX_CURRENT_UA, MAX_SETPOINT_POWER_W,
    MAX_VOLTAGE_KV, MIN_VOLTAGE_KV,
};
