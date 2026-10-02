// Deterministic Arduino I/O for the unchanged candidate's HOST_HARNESS branch.
// Inputs are simulated pin levels and time; outputs are ASCII and STEP counts.
// This header contains no serial, device, flashing, or physical timing access.
#pragma once
#include <array>
#include <cstdint>
#include <deque>
#include <sstream>
#include <string>

#define HIGH 1
#define LOW 0
#define OUTPUT 1
#define INPUT_PULLUP 2
#define F(value) value

namespace host {
inline uint64_t time_us = 0;
inline std::array<int, 32> pins{};
inline uint64_t rising_steps = 0;
inline void reset() {
  time_us = 0;
  pins.fill(HIGH);
  rising_steps = 0;
}
inline void advance(uint64_t us) { time_us += us; }
}

inline uint32_t micros() { return static_cast<uint32_t>(host::time_us); }
inline uint32_t millis() { return static_cast<uint32_t>(host::time_us / 1000); }
inline void pinMode(uint8_t, uint8_t) {}
inline int digitalRead(uint8_t pin) { return host::pins.at(pin); }
inline void digitalWrite(uint8_t pin, uint8_t level) {
  if (pin == 2 && level == HIGH && host::pins.at(pin) != HIGH) ++host::rising_steps;
  host::pins.at(pin) = level;
}

class HostSerial {
 public:
  std::deque<char> input;
  std::string output;
  void begin(unsigned long) {}
  int available() const { return static_cast<int>(input.size()); }
  int read() {
    if (input.empty()) return -1;
    const unsigned char value = input.front();
    input.pop_front();
    return value;
  }
  void feed(const std::string& text) { for (char value : text) input.push_back(value); }
  void print(unsigned char value) { print(static_cast<unsigned>(value)); }
  void print(signed char value) { print(static_cast<int>(value)); }
  template <typename T> void print(const T& value) {
    std::ostringstream stream;
    stream << value;
    output += stream.str();
  }
  template <typename T> void println(const T& value) { print(value); output += '\n'; }
  void println() { output += '\n'; }
};
inline HostSerial Serial;
