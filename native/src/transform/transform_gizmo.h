#pragma once

#include <cmath>
#include <optional>
#include <sstream>
#include <string>

// View-only normalized bounds supplied by interaction. They may exceed 0..1.
struct TransformGizmo {
  double left = 0;
  double top = 0;
  double right = 1;
  double bottom = 1;
  bool editable = false;

  bool valid() const {
    return std::isfinite(left) && std::isfinite(top) && std::isfinite(right) &&
           std::isfinite(bottom) && right > left && bottom > top &&
           std::isfinite(right - left) && std::isfinite(bottom - top);
  }
};

inline bool ParseTransformGizmo(const std::string& text, std::optional<TransformGizmo>& result) {
  std::istringstream input(text);
  input >> std::ws;
  if (input.eof()) { result.reset(); return true; }
  TransformGizmo candidate;
  int editable = 0;
  if (!(input >> candidate.left >> candidate.top >> candidate.right >> candidate.bottom >> editable) ||
      !candidate.valid() || (editable != 0 && editable != 1)) return false;
  input >> std::ws;
  if (!input.eof()) return false;
  candidate.editable = editable == 1;
  result = candidate;
  return true;
}

enum class GizmoCursor { Arrow, Move, NorthSouth, EastWest, NorthWestSouthEast, NorthEastSouthWest };

// Match interaction's handle priority and radius in logical pixels. No drag
// policy lives here: the normalized pointer stream remains authoritative.
inline GizmoCursor HitTransformGizmo(const TransformGizmo& gizmo, double x, double y,
                                    double width, double height, double dpi_scale) {
  if (!gizmo.editable || !gizmo.valid() || !std::isfinite(width) || !std::isfinite(height) ||
      width <= 0 || height <= 0 || !std::isfinite(x) || !std::isfinite(y) ||
      !std::isfinite(dpi_scale) || dpi_scale <= 0) return GizmoCursor::Arrow;
  const double left = gizmo.left * width, right = gizmo.right * width;
  const double top = gizmo.top * height, bottom = gizmo.bottom * height;
  const double center_x = left * 0.5 + right * 0.5;
  const double center_y = top * 0.5 + bottom * 0.5;
  const double radius = 8 * dpi_scale;
  GizmoCursor closest = GizmoCursor::Arrow;
  double closest_distance = radius * radius * 2 + 1;
  const auto consider = [&](double hx, double hy, GizmoCursor cursor) {
    const double dx = x - hx, dy = y - hy;
    const double distance = dx * dx + dy * dy;
    if (std::abs(dx) <= radius && std::abs(dy) <= radius && distance < closest_distance) {
      closest = cursor;
      closest_distance = distance;
    }
  };
  consider(left, top, GizmoCursor::NorthWestSouthEast);
  consider(right, top, GizmoCursor::NorthEastSouthWest);
  consider(right, bottom, GizmoCursor::NorthWestSouthEast);
  consider(left, bottom, GizmoCursor::NorthEastSouthWest);
  consider(left, center_y, GizmoCursor::EastWest);
  consider(right, center_y, GizmoCursor::EastWest);
  consider(center_x, top, GizmoCursor::NorthSouth);
  consider(center_x, bottom, GizmoCursor::NorthSouth);
  if (closest != GizmoCursor::Arrow) return closest;
  if (x >= left && x <= right && y >= top && y <= bottom) return GizmoCursor::Move;
  return GizmoCursor::Arrow;
}
