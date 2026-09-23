/**
 * Read-only interpolation between confirmed turntable samples.
 * No timer predicts future motion, scan progress or device state. Signed moves
 * keep their direction; wraparound never chooses the opposite short arc.
 */
export const ANGLE_TRANSITION_MS = 300;
export const FEEDBACK_GAP_MS = 2500;
export interface AngleFeedback {
  id: string;
  taskId: string;
  angleDeg: number;
  direction: -1 | 1;
  valid: boolean;
  running: boolean;
}

export class FeedbackAngle {
  private from = 0;
  private target = 0;
  private startedAt = 0;
  private duration = 0;
  private rawTarget = 0;
  private lastFeedbackAt = -Infinity;
  private previous: AngleFeedback | null = null;

  value(now: number): number {
    if (this.duration === 0) return this.target;
    const fraction = Math.max(0, Math.min(1, (now - this.startedAt) / this.duration));
    return this.from + (this.target - this.from) * fraction;
  }

  animating(now: number): boolean {
    return this.duration > 0 && now < this.startedAt + this.duration;
  }

  accept(feedback: AngleFeedback, now: number): void {
    if (!Number.isFinite(now)) return;
    const current = this.value(now);
    const prior = this.previous;
    const usable = feedback.valid && Number.isFinite(feedback.angleDeg);
    const snap = (angle: number) => {
      this.from = this.target = angle;
      this.startedAt = now;
      this.duration = 0;
    };
    if (!usable) {
      snap(current);
      this.previous = { ...feedback, valid: false };
      return;
    }

    const newSample = !prior || feedback.id !== prior.id;
    const interrupted = !prior?.valid || prior.taskId !== feedback.taskId
      || prior.direction !== feedback.direction
      || now - this.lastFeedbackAt > FEEDBACK_GAP_MS;
    if (newSample) this.lastFeedbackAt = now;

    if (interrupted || !feedback.running) {
      snap(feedback.angleDeg);
      this.rawTarget = feedback.angleDeg;
    } else if (Math.abs(feedback.angleDeg - this.rawTarget) > 1e-9) {
      let delta = feedback.angleDeg - this.rawTarget;
      if (feedback.direction < 0 && delta > 180) delta -= 360;
      if (feedback.direction > 0 && delta < -180) delta += 360;
      if (delta * feedback.direction < 0) {
        // A correction/re-reference is authoritative, not another full rotation.
        snap(feedback.angleDeg);
      } else {
        this.from = current;
        this.target += delta;
        this.startedAt = now;
        this.duration = ANGLE_TRANSITION_MS;
      }
      this.rawTarget = feedback.angleDeg;
    }
    // Repeated identical angles refresh liveness without restarting a transition.
    this.previous = { ...feedback };
  }
}
