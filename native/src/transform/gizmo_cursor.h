#pragma once

#include <windows.h>
#include "transform_gizmo.h"

inline HCURSOR TransformGizmoCursor(HWND hwnd, const std::optional<TransformGizmo>& gizmo,
                                  POINT client) {
  RECT rect{};
  GetClientRect(hwnd, &rect);
  const double dpi_scale = GetDpiForWindow(hwnd) / 96.0;
  const GizmoCursor cursor = gizmo ? HitTransformGizmo(*gizmo, client.x, client.y,
      rect.right - rect.left, rect.bottom - rect.top, dpi_scale) : GizmoCursor::Arrow;
  switch (cursor) {
    case GizmoCursor::Move: return LoadCursor(nullptr, IDC_SIZEALL);
    case GizmoCursor::NorthSouth: return LoadCursor(nullptr, IDC_SIZENS);
    case GizmoCursor::EastWest: return LoadCursor(nullptr, IDC_SIZEWE);
    case GizmoCursor::NorthWestSouthEast: return LoadCursor(nullptr, IDC_SIZENWSE);
    case GizmoCursor::NorthEastSouthWest: return LoadCursor(nullptr, IDC_SIZENESW);
    default: return LoadCursor(nullptr, IDC_ARROW);
  }
}
