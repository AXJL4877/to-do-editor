#pragma once

#include <cmath>
#include <sstream>
#include <string>
#include <vector>

// Soft cap matching Rust `MAX_ALIGN_GUIDES`.
inline constexpr size_t kMaxAlignGuides = 4;

// View-only smart-align guide in normalized preview-window coordinates.
struct AlignGuide {
  bool vertical = true;
  double position = 0;

  bool valid() const { return std::isfinite(position); }
};

// Empty text clears. Otherwise space-separated axis/position pairs:
// axis `0` = vertical, `1` = horizontal.
inline bool ParseAlignGuides(const std::string& text, std::vector<AlignGuide>& result) {
  std::istringstream input(text);
  input >> std::ws;
  if (input.eof()) {
    result.clear();
    return true;
  }
  std::vector<AlignGuide> guides;
  while (!input.eof()) {
    int axis = 0;
    AlignGuide guide;
    if (!(input >> axis >> guide.position) || (axis != 0 && axis != 1) || !guide.valid()) {
      return false;
    }
    guide.vertical = axis == 0;
    guides.push_back(guide);
    if (guides.size() > kMaxAlignGuides) return false;
    input >> std::ws;
  }
  result = std::move(guides);
  return true;
}
