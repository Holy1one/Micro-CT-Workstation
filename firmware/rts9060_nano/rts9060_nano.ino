/*
 * RTS9060 CT 转台控制 V2 —— Arduino Nano / ATmega328P 固件
 *
 * 硬件接线（对应本工程 kernal 原理图）：
 *   D2 --1k--> Q1(S8050) 基极，Q1 发射极接 GND，集电极接 TB6600 PUL-
 *   D3 --1k--> Q2(S8050) 基极，Q2 发射极接 GND，集电极接 TB6600 DIR-
 *   TB6600 的 PUL+/DIR+ 接控制 +5V（共阳接法），两基极各带 10k 下拉到 GND
 *   D4 <--1k-- KY-003/A3144E 的 S 脚（开漏，低电平有效），D4 开 INPUT_PULLUP
 *   D5 <-- 常开 STOP/回零按钮到 GND，D5 开 INPUT_PULLUP
 *   D6 -> 1k -> 运行 LED -> GND；D7 -> 1k -> 故障 LED -> GND；
 *   D8 -> 蜂鸣器（V1 面包板：D8 直连 J5/J_BUZZ 的 I/O，MH-FMD 模块低电平触发，D8 低=响；
 *                   V2 PCB：D8->R9(1k)->SS8050(Q3) 低边开关->BZ1，R10 下拉，D8 高=响）
 *
 * 注意：Si8050 散件存在 D331 等不同引脚排列，装板前务必用万用表确认 B/C/E。
 *
 * 串口协议：115200 8N1，一行一条命令，命令与回包均为 ASCII。
 *   HEARTBEAT <seq>                 心跳，运行中 2 s 不来就锁死，回 HBACK <seq>
 *   PING <id> / INFO <id> / STATUS <id> / GET_HALL <id>
 *   XRAY_WARNING <id> <ON|OFF>       射线警告；仅显式 OFF 才清除
 *   SET_MICROSTEPS <id> <1|2|4|8|16|32|64>
 *   REARM <id> / CLEAR_FAULT <id> / HOME <id> / ZERO <id>
 *   MOVE_ABS <id> <毫度> / MOVE_REL <id> <毫度>
 *   CAPTURE_DONE <id> / STOP <id>
 *
 * 运动语义：命令先回 ACK，脉冲跑完后进入 SETTLING；普通移动统一等待
 * MOVE_SETTLE_MS = 150 ms，且从最后一个 STEP 脉冲的下降沿起算，到期后进入
 * CAPTURE_HOLD 并回 READY_TO_CAPTURE；零位移 MOVE 从命令受理起算同样的 150 ms。
 * 该等待与角度、投影数量无关，也不再额外等待一个脉冲周期。在 CAPTURE_DONE
 * （同一个 id）之前不接受任何新的运动命令。整个过程中 STOP、心跳、按钮、串口
 * 都在轮询。150 ms 是固件预留的稳定等待，负载端是否停稳仍须图像与机械验收。
 */

// 定义 HOST_HARNESS 后可以用普通 C++ 编译器（配合 host_arduino_stubs.h）跑逻辑单测，
// 不依赖 arduino-cli，也不接硬件。
#ifndef HOST_HARNESS
#include <Arduino.h>
#include <avr/interrupt.h>
#include <avr/io.h>
#else
#include "host_arduino_stubs.h"
#endif

#include <errno.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

// ------------------------------ 引脚与参数 ------------------------------

constexpr uint8_t PIN_STEP = 2;
constexpr uint8_t PIN_DIR = 3;
constexpr uint8_t PIN_HALL = 4;
constexpr uint8_t PIN_STOP = 5;
constexpr uint8_t PIN_LED_RUN = 6;
constexpr uint8_t PIN_LED_FAULT = 7;
constexpr uint8_t PIN_BUZZER = 8;

constexpr bool BUZZER_INSTALLED = true;   // 面包板(模块)与 V2 PCB(板载 BZ1) 都装有源蜂鸣器
// 蜂鸣器有效电平（以 kernal 原理图为准，换板只改这一处）：
//   V1 面包板：J5/J_BUZZ 直连 MH-FMD 有源蜂鸣器模块，低电平触发 → 响=LOW
//   V2 PCB   ：D8→R9(1k)→Q3(SS8050 低边)+R10(10k) 下拉      → 响=HIGH（把下面改成 true）
constexpr bool BUZZER_ACTIVE_HIGH = false;
constexpr uint8_t BUZZER_ON_LEVEL  = BUZZER_ACTIVE_HIGH ? HIGH : LOW;
constexpr uint8_t BUZZER_OFF_LEVEL = BUZZER_ACTIVE_HIGH ? LOW  : HIGH;
constexpr uint8_t PROTOCOL_VERSION = 2;
#define DEVICE_TYPE_TEXT "RTS9060_NANO"
#define FIRMWARE_VERSION_TEXT "2.2.0"
#define FIRMWARE_BUILD_TEXT "20261010"
constexpr uint32_t XRAY_WARNING_PERIOD_MS = 1000;
constexpr uint32_t XRAY_WARNING_ON_MS = 500;
constexpr uint8_t STEP_IDLE = LOW;
constexpr uint8_t STEP_ACTIVE = HIGH;
constexpr uint8_t HALL_ACTIVE = LOW;   // A3144E 开漏，磁场到位拉低
constexpr uint8_t STOP_ACTIVE = LOW;   // 按钮按下接地

constexpr uint16_t MOTOR_FULL_STEPS = 200;   // 步进电机整步数/转
constexpr uint16_t GEAR_RATIO = 60;          // 转台减速比
constexpr uint16_t DEFAULT_MICROSTEPS = 8;   // TB6600 拨码需与此一致

constexpr uint16_t STEP_PULSE_US = 20;       // 脉冲高电平宽度
constexpr uint16_t DIR_SETUP_US = 100;       // 换向后到第一个脉冲
constexpr uint32_t PERIOD_FAST_US = 2000;    // 巡航 500 Hz
constexpr uint32_t PERIOD_HOME_SLOW_US = 8000;  // 回零最终复触发 125 Hz
constexpr uint32_t PERIOD_START_US = 12000;  // 起步周期
constexpr uint32_t RAMP_STEP_US = 100;       // 每个脉冲加减速的周期步长

constexpr uint32_t MOVE_SETTLE_MS = 150;       // 普通移动统一稳定时间，从末脉冲下降沿起算
constexpr uint32_t HEARTBEAT_TIMEOUT_MS = 2000;  // 上位机心跳超时
constexpr uint32_t DEBOUNCE_MS = 20;
constexpr uint32_t HOME_HOLD_MS = 2000;        // 长按 2 s 请求回零
constexpr uint32_t HOME_SEARCH_NUM = 5;        // 快速找 1.25 转
constexpr uint32_t HOME_SEARCH_DEN = 4;
constexpr uint32_t HOME_BACKOFF_MDEG = 20000;  // 仅 HOME 可反向；释放即停，20°只是故障上限
constexpr uint32_t HOME_CLEARANCE_MDEG = 250;  // 释放后再回退 0.25°，避开边沿抖动
constexpr uint32_t HOME_APPROACH_MDEG = 12000; // 同扫描方向低速复触发的有界上限
constexpr uint32_t HOME_TIMEOUT_MARGIN_MS = 5000;

constexpr int8_t HOME_DIRECTION = -1;          // 最终回零靠近与正式扫描统一使用这一方向
constexpr int32_t MAX_POSITION_PULSES = 2000000000L;
constexpr size_t COMMAND_BUFFER_SIZE = 128;
constexpr uint8_t RX_BYTES_PER_LOOP = 32;      // 每轮最多读这么多字节

enum class State : uint8_t {
  LOCKED,            // 上电/被停止后未 REARM
  IDLE,
  MOVE,              // 正在发脉冲
  SETTLING,          // 脉冲发完，等待机械稳定
  CAPTURE_HOLD,      // 等待上位机 CAPTURE_DONE
  HOME_SEARCH_FAST,  // 回零：沿扫描方向快速找进入边沿
  HOME_BACKOFF,      // 回零：唯一允许反向的阶段，找到释放边沿即停
  HOME_CLEAR,        // 回零：反向多走极小净空，避免边沿抖动
  HOME_APPROACH,     // 回零：沿扫描方向低速复触发并清零
  FAULT,
};

// -------------------------------- 全局状态 --------------------------------

static uint16_t g_microsteps = DEFAULT_MICROSTEPS;
static int32_t g_pulses = 0;              // 当前脉冲位置（相对回零点）
static int32_t g_targetPulses = 0;
static bool g_referenceValid = false;     // 位置参考是否可信
static bool g_homed = false;
static bool g_rearmed = false;

static State g_state = State::LOCKED;
static uint32_t g_activeId = 0;
static uint32_t g_captureId = 0;
static uint32_t g_stepsLeft = 0;
static uint32_t g_phaseTotal = 0;
static uint32_t g_phaseDone = 0;
static int8_t g_direction = 0;

static uint32_t g_periodUs = PERIOD_START_US;
static uint32_t g_periodMinUs = PERIOD_FAST_US;
static uint32_t g_nextStepDueUs = 0;
static uint32_t g_settleDueMs = 0;
static uint32_t g_homeStartMs = 0;
static uint32_t g_homeBudgetMs = 0;

static volatile bool g_stepHigh = false;
static volatile bool g_pulseDone = false;
static volatile uint32_t g_stepFallingUs = 0;

static bool g_heartbeatSeen = false;
static uint32_t g_lastHeartbeatMs = 0;
static bool g_xrayWarning = false;       // 只由 XRAY_WARNING 显式 ON/OFF 改变
static uint32_t g_xrayWarningStartedMs = 0;
static uint8_t g_buzzerOutput = BUZZER_OFF_LEVEL;  // 上电和未警告时保持静音电平

static bool g_buttonStable = false;
static bool g_buttonRaw = false;
static uint32_t g_buttonRawAtMs = 0;
static uint32_t g_buttonDownMs = 0;
static bool g_buttonConsumed = false;    // 这次按下已被"停止"占用
static bool g_homeRequestSent = false;
static uint32_t g_nextHomeRequestId = 1;

static char g_line[COMMAND_BUFFER_SIZE];
static size_t g_lineLen = 0;
static bool g_lineOverflow = false;

// ------------------------------ 定时器与中断 ------------------------------
// 用 Timer1 CTC 精确关掉 STEP 的高电平。20 us 如果用 digitalWrite 会被串口
// 打印拖长，TB6600 可能收不到窄脉冲，所以这里直接操作寄存器。

#ifndef HOST_HARNESS
static void setupStepTimer() {
  noInterrupts();
  TCCR1A = 0;
  TCCR1B = _BV(WGM12) | _BV(CS11);              // CTC，16 MHz/8 => 0.5 us
  TCNT1 = 0;
  OCR1A = static_cast<uint16_t>(STEP_PULSE_US * 2UL - 1UL);
  TIFR1 = _BV(OCF1A);
  TIMSK1 &= static_cast<uint8_t>(~_BV(OCIE1A));
  interrupts();
}

ISR(TIMER1_COMPA_vect) {
  PORTD &= static_cast<uint8_t>(~_BV(PD2));     // D2 = PD2
  g_stepHigh = false;
  g_stepFallingUs = micros();
  g_pulseDone = true;
  TIMSK1 &= static_cast<uint8_t>(~_BV(OCIE1A));
}
#endif   // HOST_HARNESS

static void stepPulseStart() {
#ifndef HOST_HARNESS
  noInterrupts();
  TCNT1 = 0;
  OCR1A = static_cast<uint16_t>(STEP_PULSE_US * 2UL - 1UL);
  TIFR1 = _BV(OCF1A);
  TIMSK1 |= _BV(OCIE1A);
  PORTD |= _BV(PD2);
  interrupts();
#else
  digitalWrite(PIN_STEP, STEP_ACTIVE);
  g_stepFallingUs = micros() + STEP_PULSE_US;    // 由 serviceMotion 模拟下降沿
#endif
  g_stepHigh = true;
  g_pulseDone = false;
}

static void stepForceIdle() {
#ifndef HOST_HARNESS
  noInterrupts();
  TIMSK1 &= static_cast<uint8_t>(~_BV(OCIE1A));
  PORTD &= static_cast<uint8_t>(~_BV(PD2));
  interrupts();
#else
  digitalWrite(PIN_STEP, STEP_IDLE);
#endif
  g_stepHigh = false;
  g_pulseDone = false;
  g_stepsLeft = 0;
  g_phaseTotal = 0;
  g_phaseDone = 0;
  g_direction = 0;
}

// -------------------------------- 小工具 --------------------------------

static bool reachedMs(uint32_t now, uint32_t due) {
  return static_cast<int32_t>(now - due) >= 0;
}

static bool elapsedMs(uint32_t now, uint32_t start, uint32_t span) {
  return static_cast<uint32_t>(now - start) >= span;
}

static bool isMoving() {
  return g_state != State::LOCKED && g_state != State::IDLE && g_state != State::FAULT;
}

static bool isPulsing() {
  switch (g_state) {
    case State::MOVE:
    case State::HOME_SEARCH_FAST:
    case State::HOME_BACKOFF:
    case State::HOME_CLEAR:
    case State::HOME_APPROACH:
      return true;
    default:
      return false;
  }
}

static const char *stateName() {
  switch (g_state) {
    case State::LOCKED: return "LOCKED";
    case State::IDLE: return "IDLE";
    case State::MOVE: return "MOVE";
    case State::SETTLING: return "SETTLING";
    case State::CAPTURE_HOLD: return "CAPTURE_HOLD";
    case State::HOME_SEARCH_FAST: return "HOME_SEARCH_FAST";
    case State::HOME_BACKOFF: return "HOME_BACKOFF";
    case State::HOME_CLEAR: return "HOME_CLEAR";
    case State::HOME_APPROACH: return "HOME_APPROACH";
    case State::FAULT: return "FAULT";
  }
  return "UNKNOWN";
}

// 每转脉冲数 = 电机整步 × 减速比 × 细分；角度统一用"毫度"避免浮点。
static uint32_t pulsesPerRev() {
  return static_cast<uint32_t>(MOTOR_FULL_STEPS) * GEAR_RATIO * g_microsteps;
}

static uint64_t pulsesForMdeg64(uint64_t mdeg) {
  return (static_cast<uint64_t>(pulsesPerRev()) * mdeg + 180000ULL) / 360000ULL;
}

static uint32_t pulseLimit(uint32_t mdeg) {
  uint64_t p = pulsesForMdeg64(mdeg);
  return p > 0xFFFFFFFFULL ? 0xFFFFFFFFUL : static_cast<uint32_t>(p);
}

static uint32_t homeSearchPulses() {
  uint64_t n = static_cast<uint64_t>(pulsesPerRev()) * HOME_SEARCH_NUM;
  return static_cast<uint32_t>((n + HOME_SEARCH_DEN - 1) / HOME_SEARCH_DEN);
}

// 回零整体超时：按各路段的实际速度估算，避免按最慢速度算出一个虚高的保护值。
static uint32_t homeBudgetMs() {
  uint64_t fast = static_cast<uint64_t>(homeSearchPulses()) +
                  pulseLimit(HOME_BACKOFF_MDEG) + pulseLimit(HOME_CLEARANCE_MDEG);
  uint64_t slow = pulseLimit(HOME_APPROACH_MDEG);
  uint64_t us = fast * PERIOD_FAST_US + slow * PERIOD_HOME_SLOW_US;
  return static_cast<uint32_t>((us + 999) / 1000 + HOME_TIMEOUT_MARGIN_MS);
}

static bool hallActive() { return digitalRead(PIN_HALL) == HALL_ACTIVE; }
static bool stopPressed() { return digitalRead(PIN_STOP) == STOP_ACTIVE; }

static bool parseU32(const char *s, uint32_t *out) {
  if (!s || !*s || *s == '-') return false;
  errno = 0;
  char *end = nullptr;
  unsigned long v = strtoul(s, &end, 10);
  if (errno == ERANGE || end == s || *end || v > 0xFFFFFFFFUL) return false;
  *out = static_cast<uint32_t>(v);
  return true;
}

static bool parseI32(const char *s, int32_t *out) {
  if (!s || !*s) return false;
  errno = 0;
  char *end = nullptr;
  long v = strtol(s, &end, 10);
  if (errno == ERANGE || end == s || *end || v < -2147483647L - 1L || v > 2147483647L) return false;
  *out = static_cast<int32_t>(v);
  return true;
}

static bool validMicrosteps(uint32_t m) {
  return m == 1 || m == 2 || m == 4 || m == 8 || m == 16 || m == 32 || m == 64;
}

static void printError(uint32_t id, const char *reason) {
  Serial.print("ERR ");
  if (id) { Serial.print(id); Serial.print(' '); }
  Serial.println(reason);
}

static void printAck(uint32_t id, const char *cmd) {
  Serial.print("ACK ");
  Serial.print(id);
  Serial.print(' ');
  Serial.println(cmd);
}

static void invalidatePosition() {
  g_referenceValid = false;
  g_homed = false;
  g_rearmed = false;
}

static void setBuzzerOutput(uint8_t level) {
  if (g_buzzerOutput == level) return;
  g_buzzerOutput = level;
  digitalWrite(PIN_BUZZER, level);
}

static void serviceXrayWarning() {
  uint8_t level = BUZZER_OFF_LEVEL;
  const uint32_t elapsed = static_cast<uint32_t>(millis() - g_xrayWarningStartedMs);
  if (BUZZER_INSTALLED && g_xrayWarning &&
      (elapsed % XRAY_WARNING_PERIOD_MS) < XRAY_WARNING_ON_MS) {
    level = BUZZER_ON_LEVEL;
  }
  setBuzzerOutput(level);
}

static void updateIndicators() {
  bool run;
  if (isMoving()) {
    run = (millis() / 150UL) % 2UL == 0UL;   // 运动中闪灯
  } else {
    run = (g_state == State::IDLE && g_homed && g_rearmed);
  }
  digitalWrite(PIN_LED_RUN, run ? HIGH : LOW);
  digitalWrite(PIN_LED_FAULT, (g_state == State::LOCKED || g_state == State::FAULT) ? HIGH : LOW);
}

static void clearMotion() {
  g_activeId = 0;
  g_captureId = 0;
  g_targetPulses = g_pulses;
  stepForceIdle();
}

// 统一的安全停机：清运动、作废位置参考、打印原因。
static void stopAndLock(const char *reason, uint32_t stopId, bool fault) {
  clearMotion();
  invalidatePosition();
  g_state = fault ? State::FAULT : State::LOCKED;
  if (fault) {
    Serial.print("FAULT ");
    Serial.println(reason);
  } else {
    Serial.print("STOPPED ");
    if (stopId) { Serial.print(stopId); Serial.print(' '); }
    Serial.print("POSITION_UNKNOWN reason=");
    Serial.println(reason);
  }
  updateIndicators();
}

// ------------------------------ 运动与回零 ------------------------------

static void writeDirection(int8_t dir) {
  digitalWrite(PIN_DIR, dir > 0 ? HIGH : LOW);
}

static void startPhase(State state, int8_t dir, uint32_t pulses, uint32_t periodMinUs) {
  stepForceIdle();                     // 换向前先确保 STEP 已经拉低
  g_state = state;
  g_direction = dir;
  g_stepsLeft = pulses;
  g_phaseTotal = pulses;
  writeDirection(dir);
  g_periodMinUs = periodMinUs;
  g_periodUs = PERIOD_START_US;
  g_phaseDone = 0;
  g_nextStepDueUs = micros() + DIR_SETUP_US;
  Serial.print("HOME_PHASE ");
  Serial.print(stateName());
  Serial.print(" limit=");
  Serial.println(pulses);
}

static void completeHome() {
  stepForceIdle();
  g_pulses = 0;
  g_targetPulses = 0;
  g_referenceValid = true;
  g_homed = true;
  g_state = State::IDLE;
  Serial.print("HOME_DONE ");
  Serial.print(g_activeId);
  Serial.println(" POS=0");
  g_activeId = 0;
  updateIndicators();
}

// 一个脉冲段跑完后的收尾：普通移动进入 SETTLING，回零各阶段按边沿推进。
static void finishPhase() {
  if (g_state == State::MOVE) {
    // 普通移动的正常路径已在最后一个下降沿进入稳定计时；这里只是防御性兜底
    // （例如下降沿事件被外部清除），仍然保留完整的 MOVE_SETTLE_MS 等待。
    g_state = State::SETTLING;
    g_settleDueMs = millis() + MOVE_SETTLE_MS;
    return;
  }
  if (g_state == State::HOME_CLEAR) {
    startPhase(State::HOME_APPROACH, HOME_DIRECTION,
               pulseLimit(HOME_APPROACH_MDEG), PERIOD_HOME_SLOW_US);
  } else if (g_state == State::HOME_BACKOFF) {
    stopAndLock("HOME_BACKOFF_LIMIT", g_activeId, true);
  } else if (g_state == State::HOME_APPROACH) {
    stopAndLock("HOME_APPROACH_LIMIT", g_activeId, true);
  } else {
    // 快速找边沿超出行程 = 有界故障，绝不允许无限转下去
    stopAndLock("HOME_FAST_LIMIT", g_activeId, true);
  }
}

// 普通移动的稳定计时锚点 = 最后一个 STEP 脉冲的下降沿。这里用 g_stepFallingUs
// 反推毫秒锚点，因为下降沿可能由 ISR 在本轮循环之前产生；全程使用无符号运算，
// millis/micros 回绕时仍保证"末下降沿之后至少 MOVE_SETTLE_MS"。
static void beginMoveSettling(uint32_t nowUs, uint32_t nowMs) {
  const uint32_t sinceFallingUs = nowUs - g_stepFallingUs;
  const uint32_t anchorMs = nowMs - sinceFallingUs / 1000UL;
  g_state = State::SETTLING;
  g_settleDueMs = anchorMs + MOVE_SETTLE_MS;
}

static void serviceMotion() {
  uint32_t nowUs = micros();
  uint32_t nowMs = millis();

  if (g_state == State::SETTLING) {
    if (reachedMs(nowMs, g_settleDueMs)) {
      g_state = State::CAPTURE_HOLD;
      g_captureId = g_activeId;
      Serial.print("READY_TO_CAPTURE ");
      Serial.print(g_captureId);
      Serial.print(" POS=");
      Serial.println(static_cast<long>(g_pulses));
      updateIndicators();
    }
    return;
  }
  if (!isPulsing()) return;

  if (g_state != State::MOVE && elapsedMs(nowMs, g_homeStartMs, g_homeBudgetMs)) {
    stopAndLock("HOME_TIMEOUT", g_activeId, true);
    return;
  }

  if (g_stepHigh) {
#ifdef HOST_HARNESS
    // 宿主环境没有 Timer1 中断，用软件时间模拟脉冲下降沿
    if (reachedMs(nowUs, g_stepFallingUs)) {
      digitalWrite(PIN_STEP, STEP_IDLE);
      g_stepHigh = false;
      g_pulseDone = true;
      g_stepFallingUs = nowUs;
    }
#endif
    if (g_stepHigh) return; // 高电平期间只等下降沿，不做别的事
  }

  if (g_pulseDone) {        // 中断刚拉低：末脉冲即运动终点，否则按周期排下一个脉冲
    g_pulseDone = false;
    if (g_state == State::MOVE && g_stepsLeft == 0) {
      beginMoveSettling(nowUs, nowMs);
      return;
    }
    uint32_t gap = g_periodUs > STEP_PULSE_US ? g_periodUs - STEP_PULSE_US : 1UL;
    g_nextStepDueUs = g_stepFallingUs + gap;
  }

  // 只在脉冲落下之后判断霍尔，保证边沿位置可重复
  if (g_state == State::HOME_SEARCH_FAST && hallActive()) {
    // 只在 HOME 内允许反向：退出当前感应边沿，再同向低速复触发。
    startPhase(State::HOME_BACKOFF, -HOME_DIRECTION,
               pulseLimit(HOME_BACKOFF_MDEG), PERIOD_FAST_US);
    return;
  }
  if (g_state == State::HOME_BACKOFF && !hallActive()) {
    startPhase(State::HOME_CLEAR, -HOME_DIRECTION,
               pulseLimit(HOME_CLEARANCE_MDEG), PERIOD_FAST_US);
    return;
  }
  if (g_state == State::HOME_APPROACH && hallActive()) {
    completeHome();
    return;
  }

  if (!reachedMs(nowUs, g_nextStepDueUs)) return;
  if (g_stepsLeft == 0) { finishPhase(); return; }

  // 线性加减速：头尾各 100 步以内调整周期
  uint32_t ramp = g_phaseTotal / 2UL;
  if (ramp > 100UL) ramp = 100UL;
  if (ramp == 0) ramp = 1UL;
  if (g_phaseDone < ramp && g_periodUs > g_periodMinUs) {
    g_periodUs = (g_periodUs > g_periodMinUs + RAMP_STEP_US) ? g_periodUs - RAMP_STEP_US : g_periodMinUs;
  } else if (g_stepsLeft <= ramp && g_periodUs < PERIOD_START_US) {
    g_periodUs = (g_periodUs + RAMP_STEP_US < PERIOD_START_US) ? g_periodUs + RAMP_STEP_US : PERIOD_START_US;
  }

  stepPulseStart();
  g_pulses += g_direction;
  g_phaseDone++;
  g_stepsLeft--;
}

// ------------------------------ 心跳与按钮 ------------------------------

static void serviceHeartbeat() {
  if (!isMoving()) return;
  if (!g_heartbeatSeen || elapsedMs(millis(), g_lastHeartbeatMs, HEARTBEAT_TIMEOUT_MS)) {
    stopAndLock("HEARTBEAT_TIMEOUT", g_activeId, true);
  }
}

static void serviceButton() {
  uint32_t now = millis();
  bool raw = stopPressed();
  if (raw != g_buttonRaw) { g_buttonRaw = raw; g_buttonRawAtMs = now; }
  if (g_buttonRaw != g_buttonStable && elapsedMs(now, g_buttonRawAtMs, DEBOUNCE_MS)) {
    g_buttonStable = g_buttonRaw;
    if (g_buttonStable) {
      g_buttonDownMs = now;
      g_homeRequestSent = false;
      // 按下时如果正在运动，这次按下就算"停止"用掉了，长按也不再触发回零
      g_buttonConsumed = isMoving();
      if (g_buttonConsumed) {
        stopAndLock("BUTTON", 0, false);
        Serial.println("BUTTON_STOP");
      }
    } else {
      g_buttonConsumed = false;
      g_homeRequestSent = false;
    }
  }
  if (g_buttonStable && !g_buttonConsumed && !g_homeRequestSent &&
      (g_state == State::LOCKED || g_state == State::IDLE) &&
      elapsedMs(now, g_buttonDownMs, HOME_HOLD_MS)) {
    // 长按只发"请求"，是否真的回零由操作员在上位机确认（安全联锁）
    g_homeRequestSent = true;
    g_buttonConsumed = true;
    Serial.print("HOME_REQUEST ");
    Serial.println(g_nextHomeRequestId++);
  }
}

// -------------------------------- 协议 --------------------------------

static bool takeId(uint32_t *id) {
  char *token = strtok(nullptr, " \t");
  return parseU32(token, id) && *id != 0;
}

static bool noMoreTokens() { return strtok(nullptr, " \t") == nullptr; }

static bool requireIdle(uint32_t id) {
  if (isMoving()) {
    printError(id, g_state == State::CAPTURE_HOLD ? "BUSY_CAPTURE" : "BUSY");
    return false;
  }
  if (g_state == State::FAULT) { printError(id, "FAULT_LOCKED"); return false; }
  return true;
}

static bool requireRearmed(uint32_t id) {
  if (!g_rearmed || g_state == State::LOCKED) { printError(id, "NOT_REARMED"); return false; }
  if (stopPressed()) { printError(id, "STOP_INPUT"); return false; }
  return true;
}

static bool requireHeartbeat(uint32_t id) {
  if (!g_heartbeatSeen || elapsedMs(millis(), g_lastHeartbeatMs, HEARTBEAT_TIMEOUT_MS)) {
    printError(id, "HEARTBEAT_REQUIRED");
    return false;
  }
  return true;
}

static void sendStatus(uint32_t id) {
  printAck(id, "STATUS");
  Serial.print("STATUS ");
  Serial.print(id);
  Serial.print(" state="); Serial.print(stateName());
  Serial.print(" pos="); Serial.print(static_cast<long>(g_pulses));
  Serial.print(" target="); Serial.print(static_cast<long>(g_targetPulses));
  Serial.print(" microsteps="); Serial.print(g_microsteps);
  Serial.print(" ppr="); Serial.print(pulsesPerRev());
  Serial.print(" reference="); Serial.print(g_referenceValid ? 1 : 0);
  Serial.print(" homed="); Serial.print(g_homed ? 1 : 0);
  Serial.print(" rearmed="); Serial.print(g_rearmed ? 1 : 0);
  Serial.print(" hall="); Serial.print(hallActive() ? 1 : 0);
  Serial.print(" capture_id="); Serial.println(g_captureId);
}

static void sendInfo(uint32_t id) {
  printAck(id, "INFO");
  Serial.print(F("INFO ")); Serial.print(id);
  Serial.print(F(" DEVICE=")); Serial.print(F(DEVICE_TYPE_TEXT));
  Serial.print(F(" VERSION=")); Serial.print(F(FIRMWARE_VERSION_TEXT));
  Serial.print(F(" PROTOCOL=")); Serial.print(PROTOCOL_VERSION);
  Serial.print(F(" BUILD=")); Serial.print(F(FIRMWARE_BUILD_TEXT));
  Serial.print(F(" BUZZER=")); Serial.print(BUZZER_INSTALLED ? 1 : 0);
  Serial.println(F(" CAPS=HEARTBEAT_ACK,XRAY_WARNING,RESTORE_REFERENCE,MOVE_SETTLE_150MS"));
}

static void beginMove(uint32_t id, int32_t target) {
  int64_t delta = static_cast<int64_t>(target) - g_pulses;
  g_activeId = id;
  g_captureId = 0;
  g_targetPulses = target;
  if (delta == 0) {                       // 原地不动也从命令受理起算同样的等待
    g_state = State::SETTLING;
    g_settleDueMs = millis() + MOVE_SETTLE_MS;
    return;
  }
  uint64_t magnitude = delta > 0 ? static_cast<uint64_t>(delta) : static_cast<uint64_t>(-delta);
  g_direction = delta > 0 ? 1 : -1;
  g_stepsLeft = magnitude > 0xFFFFFFFFULL ? 0xFFFFFFFFUL : static_cast<uint32_t>(magnitude);
  g_phaseTotal = g_stepsLeft;
  g_phaseDone = 0;
  g_state = State::MOVE;
  writeDirection(g_direction);
  g_periodMinUs = PERIOD_FAST_US;
  g_periodUs = PERIOD_START_US;
  g_nextStepDueUs = micros() + DIR_SETUP_US;
}

static void beginHome(uint32_t id) {
  g_activeId = id;
  g_captureId = 0;
  g_homeStartMs = millis();
  g_homeBudgetMs = homeBudgetMs();
  g_homed = false;
  g_referenceValid = false;
  if (hallActive()) {   // 已在感应区：HOME 内先反向退出，再同向低速靠近
    startPhase(State::HOME_BACKOFF, -HOME_DIRECTION,
               pulseLimit(HOME_BACKOFF_MDEG), PERIOD_FAST_US);
  } else {
    startPhase(State::HOME_SEARCH_FAST, HOME_DIRECTION,
               homeSearchPulses(), PERIOD_FAST_US);
  }
}

static void handleCommand(char *line) {
  char *command = strtok(line, " \t");
  if (!command) return;
  uint32_t id = 0;

  if (!strcmp(command, "HEARTBEAT")) {
    uint32_t seq = 0;
    char *token = strtok(nullptr, " \t");
    if (!parseU32(token, &seq) || !noMoreTokens()) { printError(0, "BAD_HEARTBEAT"); return; }
    g_heartbeatSeen = true;
    g_lastHeartbeatMs = millis();
    Serial.print("HBACK "); Serial.println(seq);
    return;
  }
  if (!strcmp(command, "PING")) {
    if (!takeId(&id) || !noMoreTokens()) { printError(0, "BAD_PING"); return; }
    printAck(id, "PING");
    Serial.print("PONG "); Serial.println(id);
  } else if (!strcmp(command, "INFO")) {
    if (!takeId(&id) || !noMoreTokens()) { printError(0, "BAD_INFO"); return; }
    sendInfo(id);
  } else if (!strcmp(command, "XRAY_WARNING")) {
    if (!takeId(&id)) { printError(0, "BAD_XRAY_WARNING"); return; }
    char *token = strtok(nullptr, " \t");
    // 保持显式的 ON/OFF 白名单，避免任意字符串改变警告状态。
    if (!token || (strcmp(token, "ON") && strcmp(token, "OFF")) || !noMoreTokens()) {
      printError(id, "BAD_XRAY_WARNING"); return;
    }
    const bool requested = !strcmp(token, "ON");
    if (requested && !g_xrayWarning) g_xrayWarningStartedMs = millis();
    g_xrayWarning = requested;
    printAck(id, "XRAY_WARNING");
    Serial.print("OK "); Serial.print(id); Serial.print(" XRAY_WARNING=");
    Serial.println(g_xrayWarning ? "ON" : "OFF");
    serviceXrayWarning();
  } else if (!strcmp(command, "STATUS")) {
    if (!takeId(&id) || !noMoreTokens()) { printError(0, "BAD_STATUS"); return; }
    sendStatus(id);
  } else if (!strcmp(command, "GET_HALL")) {
    if (!takeId(&id) || !noMoreTokens()) { printError(0, "BAD_HALL"); return; }
    printAck(id, "GET_HALL");
    Serial.print("HALL "); Serial.print(id); Serial.print(' ');
    Serial.println(hallActive() ? 1 : 0);
  } else if (!strcmp(command, "SET_MICROSTEPS")) {
    if (!takeId(&id)) { printError(0, "BAD_MICROSTEPS"); return; }
    char *token = strtok(nullptr, " \t");
    uint32_t requested = 0;
    if (!parseU32(token, &requested) || !noMoreTokens() || !validMicrosteps(requested)) {
      printError(id, "BAD_MICROSTEPS"); return;
    }
    if (!requireIdle(id)) return;
    if (requested != g_microsteps) {
      // 细分改了，位置换算全变，必须重新回零
      g_microsteps = static_cast<uint16_t>(requested);
      invalidatePosition();
      g_state = State::LOCKED;
    }
    printAck(id, "SET_MICROSTEPS");
    Serial.print("OK "); Serial.print(id);
    Serial.print(" MICROSTEPS="); Serial.print(g_microsteps);
    Serial.print(" PPR="); Serial.println(pulsesPerRev());
    updateIndicators();
  } else if (!strcmp(command, "REARM")) {
    if (!takeId(&id) || !noMoreTokens()) { printError(0, "BAD_REARM"); return; }
    if (!requireIdle(id)) return;
    if (stopPressed()) { printError(id, "STOP_INPUT"); return; }
    g_rearmed = true;
    if (g_state == State::LOCKED) g_state = State::IDLE;
    printAck(id, "REARM");
    Serial.print("OK "); Serial.print(id); Serial.println(" REARMED");
    updateIndicators();
  } else if (!strcmp(command, "CLEAR_FAULT")) {
    if (!takeId(&id) || !noMoreTokens()) { printError(0, "BAD_CLEAR_FAULT"); return; }
    if (g_state != State::FAULT) { printError(id, "NOT_FAULT"); return; }
    if (stopPressed()) { printError(id, "STOP_INPUT"); return; }
    clearMotion();
    invalidatePosition();
    g_heartbeatSeen = false;              // 必须重新建立心跳才能再动
    g_state = State::LOCKED;
    printAck(id, "CLEAR_FAULT");
    Serial.print("OK "); Serial.print(id); Serial.println(" FAULT_CLEARED");
    updateIndicators();
  } else if (!strcmp(command, "RESTORE_REFERENCE")) {
    if (!takeId(&id)) { printError(0, "BAD_REFERENCE_RESTORE"); return; }
    int32_t expected = 0;
    if (!parseI32(strtok(nullptr, " \t"), &expected) || !noMoreTokens()) {
      printError(id, "BAD_REFERENCE_RESTORE"); return;
    }
    if (!requireIdle(id) || !requireRearmed(id) || !requireHeartbeat(id)) return;
    if (g_xrayWarning || expected > 0 || expected < -MAX_POSITION_PULSES ||
        g_pulses != expected || g_targetPulses != expected || g_captureId != 0) {
      printError(id, "REFERENCE_RESTORE_POSITION_CHANGED"); return;
    }
    // PC has verified the frozen checkpoint and explicit unchanged-position
    // attestation. Restore eligibility only: never change counters or pulse STEP.
    g_referenceValid = true;
    g_homed = true;
    printAck(id, "RESTORE_REFERENCE");
    Serial.print("OK "); Serial.print(id);
    Serial.print(" REFERENCE_RESTORED POS="); Serial.println(static_cast<long>(g_pulses));
    updateIndicators();
  } else if (!strcmp(command, "HOME")) {
    if (!takeId(&id) || !noMoreTokens()) { printError(0, "BAD_HOME"); return; }
    if (!requireIdle(id) || !requireRearmed(id) || !requireHeartbeat(id)) return;
    printAck(id, "HOME");
    beginHome(id);
  } else if (!strcmp(command, "ZERO")) {
    if (!takeId(&id) || !noMoreTokens()) { printError(0, "BAD_ZERO"); return; }
    if (!requireIdle(id) || !requireRearmed(id) || !requireHeartbeat(id)) return;
    g_pulses = 0;
    g_targetPulses = 0;
    g_referenceValid = true;
    g_homed = false;                      // 仅调试用，扫描流程必须走 HOME
    printAck(id, "ZERO");
    Serial.print("OK "); Serial.print(id); Serial.println(" ZERO DEBUG_ONLY");
  } else if (!strcmp(command, "MOVE_ABS") || !strcmp(command, "MOVE_REL")) {
    bool relative = !strcmp(command, "MOVE_REL");
    if (!takeId(&id)) { printError(0, "BAD_MOVE"); return; }
    char *token = strtok(nullptr, " \t");
    int32_t mdeg = 0;
    if (!parseI32(token, &mdeg) || !noMoreTokens()) { printError(id, "BAD_ANGLE"); return; }
    if (!requireIdle(id) || !requireRearmed(id) || !requireHeartbeat(id)) return;
    if (!g_referenceValid) { printError(id, "REFERENCE_REQUIRED"); return; }

    uint64_t magnitude = mdeg < 0 ? static_cast<uint64_t>(-static_cast<int64_t>(mdeg))
                                  : static_cast<uint64_t>(mdeg);
    int64_t target;
    if (relative) {
      int64_t delta = static_cast<int64_t>(pulsesForMdeg64(magnitude));
      target = g_pulses + (mdeg < 0 ? -delta : delta);
    } else {
      // 绝对角按 0..360° 归一化，再做范围检查（先算 64 位，避免提前窄化溢出）
      uint64_t wrapped = pulsesForMdeg64(magnitude % 360000ULL);
      target = (mdeg < 0) ? -static_cast<int64_t>(wrapped) : static_cast<int64_t>(wrapped);
    }
    if (target < -MAX_POSITION_PULSES || target > MAX_POSITION_PULSES) {
      printError(id, "RANGE"); return;
    }
    int64_t delta = target - static_cast<int64_t>(g_pulses);
    int8_t requestedDirection = delta > 0 ? 1 : (delta < 0 ? -1 : 0);
    if (requestedDirection != 0 && requestedDirection != HOME_DIRECTION) {
      // HOME 的回退由内部状态机专管；普通运动永远不能反向。
      printError(id, "REVERSE_FORBIDDEN"); return;
    }
    printAck(id, command);
    beginMove(id, static_cast<int32_t>(target));
  } else if (!strcmp(command, "CAPTURE_DONE")) {
    if (!takeId(&id) || !noMoreTokens()) { printError(0, "BAD_CAPTURE_DONE"); return; }
    if (g_state != State::CAPTURE_HOLD) { printError(id, "NOT_CAPTURE_HOLD"); return; }
    if (id != g_captureId) { printError(id, "CAPTURE_ID_MISMATCH"); return; }
    printAck(id, "CAPTURE_DONE");
    Serial.print("IDLE "); Serial.print(id); Serial.println(" CAPTURE_RELEASED");
    g_captureId = 0;
    g_activeId = 0;
    g_state = State::IDLE;
    updateIndicators();
  } else if (!strcmp(command, "STOP")) {
    if (!takeId(&id) || !noMoreTokens()) { printError(0, "BAD_STOP"); return; }
    printAck(id, "STOP");
    stopAndLock("HOST_STOP", id, false);
  } else {
    printError(0, "UNKNOWN_COMMAND");
  }
}

static void serviceSerial() {
  uint8_t processed = 0;
  while (Serial.available() > 0 && processed < RX_BYTES_PER_LOOP) {
    char c = static_cast<char>(Serial.read());
    processed++;
    if (c == '\r') continue;
    if (c == '\n') {
      if (g_lineOverflow) {
        printError(0, "LINE_TOO_LONG");
      } else if (g_lineLen > 0) {
        g_line[g_lineLen] = '\0';
        handleCommand(g_line);
      }
      g_lineLen = 0;
      g_lineOverflow = false;
      continue;
    }
    if (g_lineOverflow) continue;
    if (g_lineLen + 1 >= COMMAND_BUFFER_SIZE) { g_lineOverflow = true; continue; }
    g_line[g_lineLen++] = c;
  }
}

// ------------------------------- 主流程 -------------------------------

void setup() {
  pinMode(PIN_STEP, OUTPUT);
  pinMode(PIN_DIR, OUTPUT);
  pinMode(PIN_HALL, INPUT_PULLUP);
  pinMode(PIN_STOP, INPUT_PULLUP);
  pinMode(PIN_LED_RUN, OUTPUT);
  pinMode(PIN_LED_FAULT, OUTPUT);
  pinMode(PIN_BUZZER, OUTPUT);
#ifndef HOST_HARNESS
  setupStepTimer();
#endif
  digitalWrite(PIN_STEP, STEP_IDLE);
  digitalWrite(PIN_BUZZER, BUZZER_OFF_LEVEL);

  g_buttonRaw = stopPressed();
  g_buttonStable = g_buttonRaw;
  g_buttonRawAtMs = millis();
  g_state = State::LOCKED;          // 上电就是锁定态，必须 REARM 才能动
  invalidatePosition();
  g_xrayWarning = false;
  g_xrayWarningStartedMs = millis();
  g_buzzerOutput = BUZZER_OFF_LEVEL;

  Serial.begin(115200);
  Serial.print(F("READY DEVICE=")); Serial.print(F(DEVICE_TYPE_TEXT));
  Serial.print(F(" VERSION=")); Serial.print(F(FIRMWARE_VERSION_TEXT));
  Serial.print(F(" PROTOCOL=")); Serial.print(PROTOCOL_VERSION);
  Serial.print(F(" BUILD=")); Serial.print(F(FIRMWARE_BUILD_TEXT));
  Serial.print(F(" BUZZER=")); Serial.print(BUZZER_INSTALLED ? 1 : 0);
  Serial.println(F(" CAPS=HEARTBEAT_ACK,XRAY_WARNING,RESTORE_REFERENCE,MOVE_SETTLE_150MS"));
  updateIndicators();
}

void loop() {
  // STEP 拉高期间不做任何打印或解析，保证 20 us 脉冲不被串口拖长，
  // 同时让 STOP 的响应延迟有上界。
  if (g_stepHigh) {
    serviceMotion();
  } else {
    serviceSerial();
    serviceButton();
    serviceHeartbeat();
    serviceMotion();
  }
  updateIndicators();
  serviceXrayWarning();
}
