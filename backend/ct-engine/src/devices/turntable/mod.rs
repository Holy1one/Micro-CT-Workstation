//! Turntable device module.
//!
//! `nano` implements the active RTS9060 serial contract. The module name uses
//! the device role rather than the current board so another controller can be
//! added later without changing scan-domain terminology.

mod nano;
mod planner;
pub(crate) use planner::pulses_for_forward_mdeg;

#[cfg(test)]
pub(crate) use nano::LineTransport;
#[cfg(test)]
pub(crate) use nano::tests::test_homed_nano;
#[cfg(test)]
pub(crate) use nano::tests::test_card_homed_nano;
#[cfg(test)]
pub(crate) use nano::tests::test_stopped_reference_nano;
#[cfg(test)]
pub(crate) use nano::tests::test_delayed_stop_nano;

pub use nano::{
    MoveOutcome, MoveTicket, NanoAdapter, NanoConnectionState, NanoError, NanoHealth, NanoIdentity,
    NanoStatus, BAUD_RATE, EXPECTED_DEVICE, EXPECTED_PROTOCOL,
};
pub use planner::{
    confirmed_phase_mdeg, plan_projection, plan_relative, plan_to_phase,
    verify_fixed_configuration, verify_plan_capture_hold, verify_plan_released, MotionPlan,
    FIXED_MICROSTEPS, FIXED_PULSES_PER_REV,
};
