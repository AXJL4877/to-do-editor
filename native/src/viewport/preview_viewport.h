#pragma once

#include "../transform/visual_transform.h"

// Composition frame in normalized preview-window coordinates. Legacy clients
// omit this command and keep the full-window canvas without editing decoration.
struct PreviewViewport {
  double left = 0;
  double top = 0;
  double width = 1;
  double height = 1;
  bool show_outside = true;
  bool enabled = false;

  bool valid() const {
    return std::isfinite(left) && std::isfinite(top) && std::isfinite(width) &&
           std::isfinite(height) && width > 0 && height > 0 &&
           std::isfinite(left + width) && std::isfinite(top + height);
  }
};

inline bool ParsePreviewViewport(const std::string& text, PreviewViewport& result) {
  PreviewViewport candidate;
  int outside = 0;
  std::istringstream input(text);
  if (!(input >> candidate.left >> candidate.top >> candidate.width >> candidate.height >> outside))
    return false;
  input >> std::ws;
  if (!input.eof() || !candidate.valid() || (outside != 0 && outside != 1)) return false;
  candidate.show_outside = outside != 0;
  candidate.enabled = true;
  result = candidate;
  return true;
}

inline VisualUvMap MapViewportVisual(const VisualUvMap& map, const PreviewViewport& viewport) {
  if (!viewport.enabled) return map;
  const double sx = map.scale_x / viewport.width;
  const double sy = map.scale_y / viewport.height;
  return {FiniteVisualFloat(sx), FiniteVisualFloat(sy),
          FiniteVisualFloat(map.offset_x - viewport.left * sx),
          FiniteVisualFloat(map.offset_y - viewport.top * sy)};
}
