/** User-facing numeric contracts, including camera-specific limits and minute conversion. */
import assert from "node:assert/strict";
import test from "node:test";
import { projectionError, exposureError, minutesToSeconds, secondsToMinutes } from "../src/scan-input.ts";
test("accepts every positive integer through 3600 without a step restriction", () => {
  for (let value = 1; value <= 3600; value++) assert.equal(projectionError(String(value)), null);
  for (const value of ["0", "-1", "3601", "1.5", "", "NaN"]) assert.ok(projectionError(value));
});
test("D7100 exposures include sub-millisecond and 30-second endpoints", () => {
  for (const value of ["0.125", ".125", "1", "200", "30000"]) assert.equal(exposureError(value), null);
  for (const value of ["0", "0.1249", "30000.1", "Infinity", "", "0x10"]) assert.ok(exposureError(value));
});
test("connected camera limits override the rated range", () => {
  assert.ok(exposureError("0.125", 1, 2000));
  assert.ok(exposureError("2001", 1, 2000));
  assert.equal(exposureError("2000", 1, 2000), null);
});
test("minutes use the existing seconds contract and default 10 maps to 600", () => {
  assert.equal(minutesToSeconds("10"), 600);
  assert.equal(minutesToSeconds("0.5"), 30);
  for (let seconds = 1; seconds <= 600; seconds++) assert.equal(minutesToSeconds(secondsToMinutes(seconds)), seconds);
  for (const value of ["0", "10.1", "-1", "00:10:00", "", "0.001"]) assert.equal(minutesToSeconds(value), null);
});
