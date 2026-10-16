#pragma once

#include <cstdint>
#include <limits>
#include <string>
#include <utility>

// Parse the generation separately from the rest-of-line UTF-8 path. No caller
// state changes unless the complete unsigned generation and path are valid.
inline bool ParseSourceRequest(const std::string& text, uint64_t& generation, std::string& path) {
  const size_t space = text.find(' ');
  const std::string token = text.substr(0, space);
  if (token.empty()) return false;
  uint64_t candidate = 0;
  for (const char c : token) {
    if (c < '0' || c > '9') return false;
    const uint64_t digit = c - '0';
    if (candidate > ((std::numeric_limits<uint64_t>::max)() - digit) / 10) return false;
    candidate = candidate * 10 + digit;
  }
  std::string candidate_path = space == std::string::npos ? std::string() : text.substr(space + 1);
  if (candidate_path.find_first_of(std::string("\r\n\0", 3)) != std::string::npos) return false;
  generation = candidate;
  path = std::move(candidate_path);
  return true;
}
