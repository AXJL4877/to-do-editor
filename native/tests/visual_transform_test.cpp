#include "transform/visual_transform.h"
#include "transform/align_guides.h"
#include "transform/transform_gizmo.h"
#include "transform/source_request.h"
#include "transform/frame_display_geometry.h"
#include "viewport/preview_viewport.h"

#include <cmath>
#include <cstdio>
#include <limits>
#include <vector>

namespace {
int failures = 0;

void Check(bool condition, const char* name) {
  if (!condition) {
    std::fprintf(stderr, "FAIL: %s\n", name);
    ++failures;
  }
}

bool Near(double value, double expected) {
  return std::abs(value - expected) < 1e-6;
}
}

int main() {
  PreviewViewport viewport;
  Check(ParsePreviewViewport("0.25 0.25 0.5 0.5 1", viewport), "parse editing viewport");
  Check(viewport.enabled && viewport.show_outside, "viewport enables outside editing");
  const auto viewport_map = MapViewportVisual(VisualUvMap{}, viewport);
  Check(Near(viewport_map.scale_x, 2) && Near(viewport_map.scale_y, 2) &&
        Near(viewport_map.offset_x, -0.5) && Near(viewport_map.offset_y, -0.5),
        "window UV maps back into fixed canvas UV");
  for (const char* invalid : {"0 0 0 1 1", "nan 0 1 1 1", "0 0 1 1 2", "0 0 1 1 1 extra"}) {
    const auto before = viewport;
    Check(!ParsePreviewViewport(invalid, viewport) && viewport.left == before.left &&
          viewport.width == before.width, "invalid viewport is atomic");
  }
  VisualTransform transform;
  Check(transform.valid(), "default transform is valid");
  auto map = FitVisualTransform(640, 360, 640, 360, transform);
  Check(Near(map.scale_x, 1) && Near(map.scale_y, 1) &&
        Near(map.offset_x, 0) && Near(map.offset_y, 0), "default fills equal-aspect canvas");

  map = FitVisualTransform(640, 360, 360, 360, transform);
  Check(Near(map.scale_x, 16.0 / 9) && Near(map.scale_y, 1) &&
        Near(map.offset_x, -7.0 / 18) && Near(map.offset_y, 0), "square fits centered with pillarbox");

  transform = {0.25, -0.25, 2};
  map = FitVisualTransform(640, 360, 640, 360, transform);
  Check(Near(map.scale_x, 0.5) && Near(map.scale_y, 0.5) &&
        Near(map.offset_x, 0.125) && Near(map.offset_y, 0.375), "offset uses full canvas and zoom stays centered");
  // The left edge of the shifted, scaled video maps to source u=0.
  Check(Near((-0.25 * map.scale_x) + map.offset_x, 0), "inverse mapping preserves image edge");

  VisualTransform parsed;
  Check(ParseVisualTransform("0.25 -0.5 2 0.5 3 0.25", parsed), "parse six transform fields");
  Check(Near(parsed.scale_x, 0.5) && Near(parsed.scale_y, 3) && Near(parsed.opacity, 0.25),
        "six fields preserve independent scales and opacity");
  map = FitVisualTransform(640, 360, 640, 360, parsed);
  Check(Near(map.scale_x, 1) && Near(map.scale_y, 1.0 / 6), "axis size multiplies uniform scale");
  Check(Near(map.offset_x, -0.25) && Near(map.offset_y, 0.5), "axis scaling holds transformed center");
  for (const char* invalid : {"0 0 1 0 1 1", "0 0 1 1 -1 1", "0 0 1 1 1 -0.1",
                              "0 0 1 1 1 1.1", "0 0 1 1 1", "0 0 1 1 1 1 extra"}) {
    const VisualTransform before = parsed;
    Check(!ParseVisualTransform(invalid, parsed), "reject invalid new fields and arity");
    Check(parsed.x == before.x && parsed.y == before.y && parsed.scale == before.scale &&
          parsed.scale_x == before.scale_x && parsed.scale_y == before.scale_y && parsed.opacity == before.opacity,
          "invalid new fields preserve all previous transform values");
  }
  Check(ParseVisualTransform("-0.25 1.5 0.5", parsed) &&
        Near(parsed.x, -0.25) && Near(parsed.y, 1.5) && Near(parsed.scale, 0.5), "parse signed transform");
  Check(Near(parsed.scale_x, 1) && Near(parsed.scale_y, 1) && Near(parsed.opacity, 1),
        "legacy transform resets axis multipliers and opacity to one");
  for (const char* invalid : {"0 0 0", "0 0 -1", "nan 0 1", "0 inf 1", "0 0 1 extra", "0 1"}) {
    const VisualTransform before = parsed;
    Check(!ParseVisualTransform(invalid, parsed), "reject invalid transform");
    Check(parsed.x == before.x && parsed.y == before.y && parsed.scale == before.scale,
          "invalid transform leaves previous geometry intact");
  }
  transform = {std::numeric_limits<double>::max(), 0, std::numeric_limits<double>::min()};
  map = FitVisualTransform(640, 360, 640, 360, transform);
  Check(std::isfinite(map.scale_x) && std::isfinite(map.scale_y) &&
        std::isfinite(map.offset_x) && std::isfinite(map.offset_y), "extreme finite transforms keep GPU constants finite");

  std::optional<TransformGizmo> gizmo;
  Check(ParseTransformGizmo("-0.25 0.1 1.25 0.9 1", gizmo) && gizmo && gizmo->editable,
        "gizmo accepts normalized off-canvas bounds");
  const auto saved = gizmo;
  for (const char* invalid : {"0 0 1 1 2", "0 0 0 1 1", "nan 0 1 1 1", "0 0 1 1", "0 0 1 1 1 extra"}) {
    Check(!ParseTransformGizmo(invalid, gizmo), "invalid gizmo rejected");
    Check(gizmo && Near(gizmo->left, saved->left), "invalid gizmo preserves prior state");
  }
  gizmo = TransformGizmo{0.25, 0.25, 0.75, 0.75, true};
  Check(HitTransformGizmo(*gizmo, 160, 90, 640, 360, 1) == GizmoCursor::NorthWestSouthEast,
        "top-left corner cursor");
  Check(HitTransformGizmo(*gizmo, 480, 90, 640, 360, 1) == GizmoCursor::NorthEastSouthWest,
        "top-right corner cursor");
  Check(HitTransformGizmo(*gizmo, 320, 90, 640, 360, 1) == GizmoCursor::NorthSouth,
        "top edge midpoint cursor");
  Check(HitTransformGizmo(*gizmo, 160, 180, 640, 360, 1) == GizmoCursor::EastWest,
        "left edge midpoint cursor");
  Check(HitTransformGizmo(*gizmo, 320, 180, 640, 360, 1) == GizmoCursor::Move,
        "selected body move cursor");
  Check(HitTransformGizmo(*gizmo, 150, 90, 640, 360, 1) == GizmoCursor::Arrow &&
        HitTransformGizmo(*gizmo, 150, 90, 640, 360, 2) == GizmoCursor::NorthWestSouthEast,
        "hit radius follows eight logical pixels at window DPI");
  const TransformGizmo tiny{0.25, 0.25, 0.25 + 4.0 / 640, 0.25 + 4.0 / 360, true};
  Check(HitTransformGizmo(tiny, 164, 92, 640, 360, 1) == GizmoCursor::EastWest,
        "nearest midpoint wins in a four-pixel selection");
  Check(HitTransformGizmo(tiny, 164, 90, 640, 360, 1) == GizmoCursor::NorthEastSouthWest,
        "nearest corner wins in a four-pixel selection");
  gizmo->editable = false;
  Check(HitTransformGizmo(*gizmo, 320, 180, 640, 360, 1) == GizmoCursor::Arrow,
        "locked gizmo has no edit cursor");
  Check(ParseTransformGizmo("", gizmo) && !gizmo, "empty gizmo command clears overlay");

  std::vector<AlignGuide> guides{{true, 0.5}};
  Check(ParseAlignGuides("0 0.5 1 0.25", guides) && guides.size() == 2 &&
            guides[0].vertical && Near(guides[0].position, 0.5) && !guides[1].vertical &&
            Near(guides[1].position, 0.25),
        "guides parse vertical and horizontal pairs");
  const auto saved_guides = guides;
  for (const char* invalid : {"0", "2 0.5", "0 nan", "0 0.5 1", "0 0.5 1 0.25 0 0.1 1 0.2 0 0.3"}) {
    Check(!ParseAlignGuides(invalid, guides), "invalid guides rejected");
    Check(guides.size() == saved_guides.size() && Near(guides[0].position, saved_guides[0].position),
          "invalid guides preserve prior state");
  }
  Check(ParseAlignGuides("", guides) && guides.empty(), "empty guides command clears overlay");

  uint64_t generation = 9;
  std::string path = "prior";
  Check(ParseSourceRequest("42 C:/a b.mp4", generation, path) && generation == 42 && path == "C:/a b.mp4",
        "source generation keeps spaces in source path");
  Check(ParseSourceRequest("18446744073709551615", generation, path) &&
        generation == (std::numeric_limits<uint64_t>::max)() && path.empty(), "maximum generation with clear path");
  for (const char* invalid : {"-1 file", "+1 file", "18446744073709551616 file", "bad file", " file"}) {
    const auto before = generation;
    const auto prior_path = path;
    Check(!ParseSourceRequest(invalid, generation, path) && generation == before && path == prior_path,
          "source request parse failure leaves generation and path untouched");
  }
  DecoderFrame frame;
  frame.width = 720;
  frame.height = 576;
  frame.pixel_aspect_ratio = 16.f / 15;
  auto display = DecodedFrameDisplayGeometry(frame, 720, 576);
  Check(display && Near(display->aspect, 4.0 / 3), "SAR 16:15 changes 720x576 display to 4:3");
  frame.crop_x = 8;
  frame.crop_y = 4;
  display = DecodedFrameDisplayGeometry(frame, 720, 576);
  Check(display && Near(display->aspect, 712.0 * (16.f / 15) / 572),
        "aperture crop participates in authoritative display aspect");
  Check(!DecodedFrameDisplayGeometry(frame, 0, 576), "invalid texture size has no display aspect");
  frame.crop_x = 720;
  Check(!DecodedFrameDisplayGeometry(frame, 720, 576), "empty visible aperture has no display aspect");
  return failures ? 1 : 0;
}
