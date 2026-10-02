# Turntable adapter and phase planner

`nano.rs` owns one serial worker, the versioned Nano protocol, heartbeat,
emergency STOP, HOME, arrival and matching CAPTURE_DONE. `planner.rs` consumes
confirmed STATUS, fixes the candidate scale to 8 microsteps / 96,000 pulses per
revolution and maps positive user phases to bounded negative MOVE_REL. Device
modules do not call each other; cross-device HOME safety remains in the engine.

Public planning functions are `plan_relative` (positive `(0, 360]` degrees),
`plan_to_phase` (`[0, 360)` degrees), and `plan_projection` (existing negative
single-circle scan angles). Angles use integer millidegrees. Plans retain the
unwrapped source/target pulse counts and the quantized positive phase. Targets
already at the desired phase are no-ops for manual controls; scan code can execute
REL 0 to acquire a fresh CAPTURE_HOLD ticket. Very small relative angles that
quantize to zero pulses are rejected with a reason.

`execute_plan` rechecks source STATUS on the serial owner, sends MOVE_REL and
requires arrival/ticket/STATUS/target agreement. `verify_plan_capture_hold` and
`verify_plan_released` let the engine/scan preserve this evidence through capture
and release. Matching CAPTURE_DONE must restore IDLE at the same confirmed
unwrapped position before a motion operation reports completion. Ordinary
reverse motion is rejected before sending by both adapter and planner; legacy
MOVE_ABS remains for bounded callers and rejects a target above current position.
HOME retains the candidate's internal bounded opposite-direction exception.

The maintained candidate is `firmware/rts9060_nano/`; it is not certified against
current flash or physical motion. Tests use scripted `LineTransport` through the
same serial worker, plus a C++ harness including the actual firmware candidate.
Neither test path opens a physical device. Hardware acceptance remains required.
