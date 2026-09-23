/** Observable angle-following behavior, independent of React or WebGL. */
import assert from "node:assert/strict";
import test from "node:test";
import { FeedbackAngle, ANGLE_TRANSITION_MS } from "../src/scene/angle-feedback.ts";
const sample = (angleDeg, id, extra = {}) => ({ angleDeg, id, taskId: "scan-a", direction: -1, valid: true, running: true, ...extra });
const close = (actual, expected) => assert.ok(Math.abs(actual - expected) < 1e-8, `${actual} != ${expected}`);

test("each confirmed step advances at constant speed and never overshoots", () => {
 const motion=new FeedbackAngle(); motion.accept(sample(0,"0"),0); motion.accept(sample(-90,"1"),100);
 for(const [time,angle] of [[100,0],[175,-22.5],[250,-45],[325,-67.5],[400,-90],[5000,-90]]) close(motion.value(time),angle);
 assert.equal(motion.animating(100+ANGLE_TRANSITION_MS),false);
});
test("duplicate angles do not keep restarting the transition", () => {
 const motion=new FeedbackAngle();motion.accept(sample(0,"0"),0);motion.accept(sample(-90,"1"),100);
 motion.accept(sample(-90,"2"),175);close(motion.value(250),-45);close(motion.value(400),-90);
});
test("new feedback retargets from the current displayed position without a jump", () => {
 const motion=new FeedbackAngle();motion.accept(sample(0,"0"),0);motion.accept(sample(-90,"1"),100);
 motion.accept(sample(-180,"2"),250);close(motion.value(250),-45);close(motion.value(400),-112.5);close(motion.value(550),-180);
});
test("signed scan direction survives zero crossing and skipped samples", () => {
 const negative=new FeedbackAngle();negative.accept(sample(-350,"0"),0);negative.accept(sample(0,"1"),100);
 close(negative.value(250),-355);close(negative.value(400),-360);
 const skipped=new FeedbackAngle();skipped.accept(sample(0,"0"),0);skipped.accept(sample(-270,"1"),100);
 close(skipped.value(250),-135);
 const positive=new FeedbackAngle();positive.accept(sample(350,"0",{direction:1}),0);positive.accept(sample(10,"1",{direction:1}),100);
 close(positive.value(250),360);close(positive.value(400),370);
});
test("pause, stop and completion settle immediately at confirmed position", () => {
 for(const angle of [-90,0]) {
  const motion=new FeedbackAngle();motion.accept(sample(0,"0"),0);motion.accept(sample(-90,"1"),100);
  motion.accept(sample(angle,"2",{running:false}),175);close(motion.value(175),angle);close(motion.value(2000),angle);
  assert.equal(motion.animating(175),false);
 }
});
test("loss or unknown feedback freezes the current pose and never guesses a target", () => {
 const motion=new FeedbackAngle();motion.accept(sample(0,"0"),0);motion.accept(sample(-90,"1"),100);
 motion.accept(sample(-180,"2",{valid:false}),175);close(motion.value(175),-22.5);close(motion.value(2000),-22.5);
 motion.accept(sample(-180,"3"),2100);close(motion.value(2100),-180);
});
test("resuming follows the next feedback from the held position at constant speed", () => {
 const motion=new FeedbackAngle();motion.accept(sample(-90,"0",{running:false}),0);
 motion.accept(sample(-100,"1"),100);close(motion.value(100),-90);close(motion.value(250),-95);close(motion.value(400),-100);
});
test("long feedback gaps and new tasks re-anchor instead of replaying unobserved motion", () => {
 const motion=new FeedbackAngle();motion.accept(sample(0,"0"),0);motion.accept(sample(-90,"1"),3000);
 close(motion.value(3000),-90);
 motion.accept(sample(0,"2",{taskId:"scan-b"}),3100);close(motion.value(3100),0);
});
test("reference corrections and non-finite samples cannot cause spurious rotations", () => {
 const motion=new FeedbackAngle();motion.accept(sample(-90,"0"),0);motion.accept(sample(-80,"1"),100);
 close(motion.value(100),-80);
 motion.accept(sample(NaN,"2"),150);close(motion.value(500),-80);
});
