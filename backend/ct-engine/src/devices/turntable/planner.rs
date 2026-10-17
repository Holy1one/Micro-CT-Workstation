//! Plans one prescribed-direction motion from confirmed unwrapped Nano pulses.
//! Inputs are verified STATUS and user/scan phase; outputs are bounded MOVE_REL.
//! This module neither accesses devices nor decides cross-device safety policy.
use super::nano::{MoveTicket, NanoError, NanoStatus};
use serde::Serialize;

pub const FIXED_MICROSTEPS: u32 = 8;
pub const FIXED_PULSES_PER_REV: u32 = 96_000;
pub const MAX_POSITION_PULSES: i64 = 2_000_000_000;
const CIRCLE_MDEG: i32 = 360_000;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MotionPlan {
    pub from_pulses: i64,
    pub target_pulses: i64,
    pub protocol_delta_mdeg: i32,
    /// Positive phase along the prescribed direction, before pulse quantization.
    pub requested_phase_mdeg: i32,
    /// Confirmed-pulse target phase in the same positive UI convention.
    pub quantized_phase_mdeg: f64,
}

impl MotionPlan {
    pub fn is_noop(&self) -> bool {
        self.from_pulses == self.target_pulses
    }

    pub(crate) fn validate(&self) -> Result<(), NanoError> {
        require_delta(self.protocol_delta_mdeg)?;
        if !(0..CIRCLE_MDEG).contains(&self.requested_phase_mdeg)
            || !(-MAX_POSITION_PULSES..=MAX_POSITION_PULSES).contains(&self.from_pulses)
            || !(-MAX_POSITION_PULSES..=MAX_POSITION_PULSES).contains(&self.target_pulses)
            || self.target_pulses
                != self.from_pulses - pulses_for_forward_mdeg(-self.protocol_delta_mdeg)
            || self.quantized_phase_mdeg != confirmed_phase_mdeg(self.target_pulses)
        {
            return Err(NanoError::Safety(
                "invalid or changed prescribed-direction motion plan".into(),
            ));
        }
        Ok(())
    }
}

/// Pulse scale is fixed to the recovered candidate; identity strings do not prove flash equality.
pub fn verify_fixed_configuration(status: &NanoStatus) -> Result<(), NanoError> {
    if status.microsteps != FIXED_MICROSTEPS || status.pulses_per_rev != FIXED_PULSES_PER_REV {
        return Err(NanoError::Safety(format!(
            "unsupported Nano pulse scale: microsteps={} ppr={}; expected 8/96000",
            status.microsteps, status.pulses_per_rev
        )));
    }
    if !(-MAX_POSITION_PULSES..=MAX_POSITION_PULSES).contains(&status.position_pulses)
        || !(-MAX_POSITION_PULSES..=MAX_POSITION_PULSES).contains(&status.target_pulses)
    {
        return Err(NanoError::Safety(
            "Nano position exceeds the recovered candidate range".into(),
        ));
    }
    Ok(())
}

pub(crate) fn verify_ready(status: &NanoStatus) -> Result<(), NanoError> {
    verify_fixed_configuration(status)?;
    if status.state != "IDLE"
        || status.capture_id != 0
        || !status.reference_valid
        || !status.homed
        || !status.rearmed
        || status.position_pulses != status.target_pulses
    {
        return Err(NanoError::Safety(
            "motion requires confirmed IDLE, HOME and valid reference".into(),
        ));
    }
    Ok(())
}

pub(crate) fn require_delta(delta_mdeg: i32) -> Result<(), NanoError> {
    if !(-CIRCLE_MDEG..=0).contains(&delta_mdeg) {
        return Err(NanoError::Safety("ordinary motion must follow the negative protocol direction and stay within one circle".into()));
    }
    Ok(())
}

pub(crate) fn pulses_for_forward_mdeg(mdeg: i32) -> i64 {
    (i64::from(FIXED_PULSES_PER_REV) * i64::from(mdeg) + 180_000) / i64::from(CIRCLE_MDEG)
}

/// UI phase is positive forward; the device position remains signed and unwrapped.
pub fn confirmed_phase_mdeg(position_pulses: i64) -> f64 {
    (-position_pulses.rem_euclid(i64::from(FIXED_PULSES_PER_REV)))
        .rem_euclid(i64::from(FIXED_PULSES_PER_REV)) as f64
        * f64::from(CIRCLE_MDEG)
        / f64::from(FIXED_PULSES_PER_REV)
}

fn plan_pulses(
    status: &NanoStatus,
    delta_pulses: i64,
    phase_mdeg: i32,
) -> Result<MotionPlan, NanoError> {
    verify_ready(status)?;
    let protocol_delta_mdeg = -((delta_pulses * i64::from(CIRCLE_MDEG)
        + i64::from(FIXED_PULSES_PER_REV) / 2)
        / i64::from(FIXED_PULSES_PER_REV)) as i32;
    let target_pulses = status
        .position_pulses
        .checked_sub(delta_pulses)
        .ok_or_else(|| NanoError::Safety("Nano unwrapped position overflow".into()))?;
    let plan = MotionPlan {
        from_pulses: status.position_pulses,
        target_pulses,
        protocol_delta_mdeg,
        requested_phase_mdeg: phase_mdeg,
        quantized_phase_mdeg: confirmed_phase_mdeg(target_pulses),
    };
    plan.validate()?;
    Ok(plan)
}

pub fn plan_relative(status: &NanoStatus, forward_mdeg: i32) -> Result<MotionPlan, NanoError> {
    verify_ready(status)?;
    if !(1..=CIRCLE_MDEG).contains(&forward_mdeg) {
        return Err(NanoError::Safety(
            "relative angle must be positive and no greater than 360 degrees".into(),
        ));
    }
    let delta = pulses_for_forward_mdeg(forward_mdeg);
    if delta == 0 {
        return Err(NanoError::Safety(
            "requested relative angle is smaller than one quantized pulse".into(),
        ));
    }
    let phase = ((confirmed_phase_mdeg(status.position_pulses).round() as i32) + forward_mdeg)
        .rem_euclid(CIRCLE_MDEG);
    plan_pulses(status, delta, phase)
}

pub fn plan_to_phase(status: &NanoStatus, phase_mdeg: i32) -> Result<MotionPlan, NanoError> {
    if !(0..CIRCLE_MDEG).contains(&phase_mdeg) {
        return Err(NanoError::Safety(
            "absolute phase must lie in [0, 360) degrees".into(),
        ));
    }
    verify_ready(status)?;
    let ppr = i64::from(FIXED_PULSES_PER_REV);
    let phase = pulses_for_forward_mdeg(phase_mdeg).rem_euclid(ppr);
    let current = (-status.position_pulses).rem_euclid(ppr);
    let delta = (phase - current).rem_euclid(ppr);
    plan_pulses(status, delta, phase_mdeg)
}

pub fn plan_projection(
    status: &NanoStatus,
    nominal_negative_mdeg: i32,
) -> Result<MotionPlan, NanoError> {
    if !(-CIRCLE_MDEG + 1..=0).contains(&nominal_negative_mdeg) {
        return Err(NanoError::Safety(
            "projection nominal angle must preserve the single-circle negative sequence".into(),
        ));
    }
    plan_to_phase(status, -nominal_negative_mdeg)
}

pub fn verify_plan_capture_hold(
    plan: &MotionPlan,
    ticket: &MoveTicket,
    status: &NanoStatus,
) -> Result<(), NanoError> {
    plan.validate()?;
    verify_fixed_configuration(status)?;
    if status.state != "CAPTURE_HOLD"
        || ticket.command_id == 0
        || status.capture_id != ticket.command_id
        || ticket.angle_mdeg != plan.protocol_delta_mdeg
        || ticket.position_pulses != plan.target_pulses
        || status.position_pulses != plan.target_pulses
        || status.target_pulses != plan.target_pulses
        || !status.reference_valid
        || !status.homed
        || !status.rearmed
    {
        return Err(NanoError::Safety(
            "arrival/ticket/STATUS does not match the planned unwrapped target".into(),
        ));
    }
    Ok(())
}

pub fn verify_plan_released(plan: &MotionPlan, status: &NanoStatus) -> Result<(), NanoError> {
    plan.validate()?;
    verify_ready(status)?;
    if status.position_pulses != plan.target_pulses {
        return Err(NanoError::Safety(
            "capture release changed the confirmed unwrapped target".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn status(position_pulses: i64) -> NanoStatus {
        NanoStatus {
            state: "IDLE".into(),
            position_pulses,
            target_pulses: position_pulses,
            microsteps: 8,
            pulses_per_rev: 96000,
            reference_valid: true,
            homed: true,
            rearmed: true,
            hall_active: false,
            capture_id: 0,
        }
    }
    #[test]
    fn capture_hold_evidence_rejects_a_wrong_ticket_and_stale_or_reused_status() {
        let source = status(0);
        let plan = plan_to_phase(&source, 90_000).unwrap();
        let ticket = MoveTicket {
            command_id: 7,
            angle_mdeg: plan.protocol_delta_mdeg,
            position_pulses: plan.target_pulses,
        };
        let ready = NanoStatus {
            state: "CAPTURE_HOLD".into(),
            position_pulses: plan.target_pulses,
            target_pulses: plan.target_pulses,
            capture_id: 7,
            ..source.clone()
        };
        verify_plan_capture_hold(&plan, &ticket, &ready).unwrap();
        // A different ticket, a drifted/stale position or a released (IDLE)
        // status all fail closed instead of passing off old evidence.
        assert!(verify_plan_capture_hold(
            &plan,
            &MoveTicket {
                command_id: 8,
                ..ticket.clone()
            },
            &ready,
        )
        .is_err());
        assert!(verify_plan_capture_hold(
            &plan,
            &ticket,
            &NanoStatus {
                position_pulses: ready.position_pulses + 1,
                ..ready.clone()
            },
        )
        .is_err());
        assert!(verify_plan_capture_hold(
            &plan,
            &ticket,
            &NanoStatus {
                state: "IDLE".into(),
                capture_id: 0,
                ..ready.clone()
            },
        )
        .is_err());
    }

    #[test]
    fn relative_full_circle_retains_unwrapped_displacement() {
        let plan = plan_relative(&status(-800), 360000).unwrap();
        assert_eq!(plan.protocol_delta_mdeg, -360000);
        assert_eq!(plan.target_pulses, -96800);
        assert_eq!(plan.quantized_phase_mdeg, 3000.0);
    }
    #[test]
    fn absolute_crosses_zero_only_along_prescribed_direction() {
        let plan = plan_to_phase(&status(-91200), 18000).unwrap();
        assert_eq!(plan.protocol_delta_mdeg, -36000);
        assert_eq!(plan.target_pulses, -100800);
        assert!(plan_to_phase(&status(-100800), 18000).unwrap().is_noop());
    }
    #[test]
    fn consecutive_scans_reuse_home_and_advance_to_next_zero_phase() {
        let first_last = status(-95467);
        let second_zero = plan_projection(&first_last, 0).unwrap();
        assert_eq!(second_zero.target_pulses, -96000);
        let second_last = plan_projection(&status(-96000), -358000).unwrap();
        assert_eq!(second_last.target_pulses, -191467);
        assert_eq!(second_last.quantized_phase_mdeg, 358001.25);
    }
    #[test]
    fn planner_rejects_reverse_multi_circle_unknown_reference_and_wrong_scale() {
        let ready = status(0);
        for angle in [-1000, 0, 360001, i32::MAX] {
            assert!(plan_relative(&ready, angle).is_err());
        }
        for angle in [-1, 360000] {
            assert!(plan_to_phase(&ready, angle).is_err());
        }
        for angle in [1, -360000, i32::MIN] {
            assert!(plan_projection(&ready, angle).is_err());
        }
        let mut bad = ready.clone();
        bad.microsteps = 16;
        assert!(plan_relative(&bad, 90000).is_err());
        bad = ready.clone();
        bad.reference_valid = false;
        assert!(plan_relative(&bad, 90000).is_err());
        bad = ready;
        bad.position_pulses = -MAX_POSITION_PULSES;
        bad.target_pulses = bad.position_pulses;
        assert!(plan_relative(&bad, 360000).is_err());
    }
}
