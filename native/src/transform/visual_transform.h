#pragma once

#include <algorithm>
#include <cmath>
#include <limits>
#include <sstream>
#include <string>

// Offsets are fractions of the whole preview canvas. Scale is relative to the
// source's centered aspect fit, so changing it does not change its center.
struct VisualTransform {
  double x = 0;
  double y = 0;
  double scale = 1;
  double scale_x = 1;
  double scale_y = 1;
  double opacity = 1;

  bool valid() const {
    return std::isfinite(x) && std::isfinite(y) && std::isfinite(scale) && scale > 0 &&
           std::isfinite(scale_x) && scale_x > 0 && std::isfinite(scale_y) && scale_y > 0 &&
           std::isfinite(opacity) && opacity >= 0 && opacity <= 1;
  }
};

inline bool ParseVisualTransform(const std::string& text, VisualTransform& result) {
  VisualTransform candidate;
  std::istringstream input(text);
  if (!(input >> candidate.x >> candidate.y >> candidate.scale)) return false;
  input >> std::ws;
  // The old three-field command always resets added fields to their defaults.
  if (!input.eof()) {
    if (!(input >> candidate.scale_x >> candidate.scale_y >> candidate.opacity)) return false;
    input >> std::ws;
  }
  if (!input.eof() || !candidate.valid()) return false;
  result = candidate;
  return true;
}

// Inverse destination-to-source mapping, evaluated by the pixel shader while
// the D3D viewport stays on the canvas. It supports negative positions and large
// zooms without exceeding D3D's viewport bounds.
struct VisualUvMap {
  float scale_x = 1;
  float scale_y = 1;
  float offset_x = 0;
  float offset_y = 0;
};

inline float FiniteVisualFloat(double value) {
  const double limit = (std::numeric_limits<float>::max)();
  if (std::isnan(value)) return 0.f;
  return static_cast<float>((std::max)(-limit, (std::min)(limit, value)));
}

inline VisualUvMap FitVisualTransform(double canvas_width, double canvas_height,
                                     double display_width, double display_height,
                                     const VisualTransform& transform) {
  if (!std::isfinite(canvas_width) || !std::isfinite(canvas_height) ||
      !std::isfinite(display_width) || !std::isfinite(display_height) ||
      canvas_width <= 0 || canvas_height <= 0 || display_width <= 0 || display_height <= 0 ||
      !transform.valid()) return {};
  const double fit = (std::min)(canvas_width / display_width, canvas_height / display_height);
  const double scale_x = (canvas_width / display_width / fit) / transform.scale / transform.scale_x;
  const double scale_y = (canvas_height / display_height / fit) / transform.scale / transform.scale_y;
  return {FiniteVisualFloat(scale_x), FiniteVisualFloat(scale_y),
          FiniteVisualFloat(0.5 - (0.5 + transform.x) * scale_x),
          FiniteVisualFloat(0.5 - (0.5 + transform.y) * scale_y)};
}
