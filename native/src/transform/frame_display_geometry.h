#pragma once

#include "../decoder.h"
#include <algorithm>
#include <cmath>
#include <optional>

struct FrameDisplayGeometry {
  float crop_x;
  float crop_y;
  float visible_width;
  float visible_height;
  float display_width;
  double aspect;
};

// One definition for shader UV crop, aspect fitting, and reported geometry.
inline std::optional<FrameDisplayGeometry> DecodedFrameDisplayGeometry(
    const DecoderFrame& frame, UINT texture_width, UINT texture_height) {
  if (!texture_width || !texture_height || frame.width < 1 || frame.height < 1 ||
      !std::isfinite(frame.crop_x) || !std::isfinite(frame.crop_y)) return {};
  const float crop_x = (std::max)(0.f, frame.crop_x);
  const float crop_y = (std::max)(0.f, frame.crop_y);
  const float visible_width = (std::min)(static_cast<float>(frame.width), texture_width - crop_x);
  const float visible_height = (std::min)(static_cast<float>(frame.height), texture_height - crop_y);
  if (visible_width < 1 || visible_height < 1) return {};
  const float ratio = std::isfinite(frame.pixel_aspect_ratio) && frame.pixel_aspect_ratio > 0
      ? frame.pixel_aspect_ratio : 1.f;
  const float display_width = visible_width * ratio;
  const double aspect = static_cast<double>(display_width) / visible_height;
  if (!std::isfinite(aspect) || aspect <= 0) return {};
  return FrameDisplayGeometry{crop_x, crop_y, visible_width, visible_height, display_width, aspect};
}
