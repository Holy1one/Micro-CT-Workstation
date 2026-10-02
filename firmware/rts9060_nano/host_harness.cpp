// Executes the actual recovered firmware with simulated Arduino pins and time.
// All assertions concern protocol/state behavior, never physical accuracy or ISR timing.
#define HOST_HARNESS
#include "rts9060_nano.ino"
#include <functional>
#include <iostream>
#include <stdexcept>

static unsigned checks = 0;
static void require(bool condition, const char* description) {
  ++checks;
  if (!condition) throw std::runtime_error(description);
}
static void reset() {
  host::reset(); Serial = HostSerial{};
  clearMotion();
  g_pulses = 0; g_targetPulses = 0; g_microsteps = DEFAULT_MICROSTEPS;
  g_heartbeatSeen = false; g_lineLen = 0; g_lineOverflow = false;
  g_buttonConsumed = false; g_homeRequestSent = false;
  setup();
}
static void command(const std::string& text) {
  Serial.feed(text + '\n');
  for (unsigned attempt = 0; attempt < 1000 && Serial.available(); ++attempt) {
    loop(); host::advance(20);
  }
  require(Serial.available() == 0, "serial parser did not drain bounded input");
}
static void pump_until(State target, const std::function<void()>& pins = [] {}) {
  uint32_t heartbeat_at = millis();
  for (unsigned iterations = 0; iterations < 1000000; ++iterations) {
    if (g_state == target) return;
    pins();
    if (static_cast<uint32_t>(millis() - heartbeat_at) >= 500) {
      command("HEARTBEAT 42"); heartbeat_at = millis();
    }
    loop();
    if (g_stepHigh) host::advance(STEP_PULSE_US);
    else if (isPulsing()) {
      const int32_t gap = static_cast<int32_t>(g_nextStepDueUs - micros());
      host::advance(gap > 0 ? static_cast<uint32_t>(gap) : 1);
    } else host::advance(1000);
  }
  throw std::runtime_error("firmware did not reach bounded terminal state");
}
static void rearm() { command("HEARTBEAT 1"); command("REARM 1"); }
static void home() {
  host::pins[PIN_HALL] = LOW;
  command("HOME 2");
  require(g_state == State::HOME_BACKOFF && g_direction == 1, "HOME backoff exception missing");
  require(g_phaseTotal == pulseLimit(20000), "HOME backoff bound changed");
  pump_until(State::IDLE, [] {
    if (g_state == State::HOME_BACKOFF && g_phaseDone >= 2) host::pins[PIN_HALL] = HIGH;
    if (g_state == State::HOME_CLEAR) {
      require(g_direction == 1 && g_phaseTotal == pulseLimit(250), "HOME clearance changed");
    }
    if (g_state == State::HOME_APPROACH) {
      require(g_direction == -1 && g_phaseTotal == pulseLimit(12000), "HOME approach changed");
      if (g_phaseDone >= 4) host::pins[PIN_HALL] = LOW;
    }
  });
  require(g_homed && g_referenceValid && g_rearmed && g_pulses == 0, "HOME did not establish reference");
  require(Serial.output.find("HOME_DONE 2 POS=0") != std::string::npos, "HOME terminal ID missing");
}
int main() {
  try {
    reset();
    require(g_state == State::LOCKED && !g_referenceValid, "power-on must be locked");
    require(pulsesPerRev() == 96000 && DEFAULT_MICROSTEPS == 8, "fixed pulse scale changed");
    rearm(); home();
    command("MOVE_REL 3 90000");
    require(g_state == State::IDLE && g_pulses == 0, "ordinary reverse move was accepted");
    require(Serial.output.find("ERR 3 REVERSE_FORBIDDEN") != std::string::npos, "reverse rejection absent");
    command("MOVE_ABS 4 -360000");
    require(g_state == State::SETTLING && g_targetPulses == 0, "ABS must normalize a full circle to zero");
    pump_until(State::CAPTURE_HOLD);
    require(g_captureId == 4 && g_pulses == 0, "zero-position hold has wrong ticket");
    command("CAPTURE_DONE 99");
    require(g_state == State::CAPTURE_HOLD && g_captureId == 4, "wrong ticket released hold");
    command("MOVE_REL 5 -90000");
    require(g_state == State::CAPTURE_HOLD, "motion allowed before capture release");
    command("CAPTURE_DONE 4");
    require(g_state == State::IDLE && g_captureId == 0, "matching capture release failed");
    const uint64_t before = host::rising_steps;
    command("MOVE_REL 6 -360000");
    require(g_state == State::MOVE && g_direction == -1 && g_targetPulses == -96000, "REL full-circle displacement lost");
    pump_until(State::CAPTURE_HOLD);
    require(g_pulses == -96000 && host::rising_steps - before == 96000, "full-circle pulse count mismatch");
    require(g_captureId == 6 && Serial.output.find("READY_TO_CAPTURE 6 POS=-96000") != std::string::npos,
            "arrival evidence does not carry move ID");
    command("CAPTURE_DONE 6");
    command("MOVE_ABS 7 0");
    require(g_state == State::IDLE && g_pulses == -96000, "ABS improperly wrapped forward from previous turn");
    require(Serial.output.find("ERR 7 REVERSE_FORBIDDEN") != std::string::npos, "ABS reverse protection absent");
    command("XRAY_WARNING 8 ON"); command("STOP 9");
    require(g_state == State::LOCKED && !g_homed && !g_referenceValid && !g_rearmed, "STOP retained reference");
    require(g_xrayWarning, "STOP must not silently clear X-ray warning");
    command("XRAY_WARNING 10 OFF");
    require(!g_xrayWarning, "explicit warning OFF failed");
    reset(); rearm(); host::pins[PIN_HALL] = LOW; command("HOME 11");
    pump_until(State::FAULT);
    require(!g_referenceValid && Serial.output.find("HOME_BACKOFF_LIMIT") != std::string::npos,
            "stuck Hall backoff must stop at bound");
    reset(); rearm(); host::pins[PIN_HALL] = HIGH; command("HOME 12");
    require(g_direction == -1 && g_phaseTotal == 120000, "HOME search bound changed");
    pump_until(State::FAULT);
    require(Serial.output.find("HOME_FAST_LIMIT") != std::string::npos, "missing Hall must stop at search bound");
    std::cout << "PASS actual candidate host harness: " << checks << " assertions\n";
    return 0;
  } catch (const std::exception& error) {
    std::cerr << "FAIL actual candidate host harness: " << error.what() << '\n';
    return 1;
  }
}
