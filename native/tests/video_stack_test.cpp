// Exercise the real command handlers without a running window thread so the
// pending-decode interval and stale notifications are deterministic.
#define main PreviewHostMain
#include "../src/preview_host_main.cpp"
#undef main

#include <chrono>

namespace {
int failures = 0;

void Check(bool condition, const char* message) {
  if (!condition) {
    std::fprintf(stderr, "FAIL: %s\n", message);
    ++failures;
  }
}

std::shared_ptr<const std::vector<VideoFrame>> WaitForStack(VideoEngine& engine,
                                                          uint64_t generation,
                                                          size_t count) {
  const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(10);
  while (std::chrono::steady_clock::now() < deadline) {
    auto stack = engine.latest();
    if (stack && stack->size() == count && stack->front().generation == generation &&
        stack->front().exact) return stack;
    std::this_thread::sleep_for(std::chrono::milliseconds(1));
  }
  Check(false, "layer stack completes before timeout");
  return {};
}

void TestPendingPresentation() {
  auto visible = std::make_shared<std::vector<VideoFrame>>(2);
  g_frames = visible;
  const TransformGizmo gizmo{0.1, 0.2, 0.8, 0.9, true};
  g_presenter.setGizmo(gizmo);
  g_presenter.setGuides({{true, 0.5}});
  g_pending_layers = {{VideoFrame::Kind::Video, L"bottom.mp4", 1.0, {}}, {VideoFrame::Kind::Video, L"top.mp4", 2.0, {0.1, 0, 1}}};
  g_layers_configuring = true;
  CmdLayersCommit();
  Check(g_frames == visible, "layer update retains visible stack while decoding is pending");
  Check(g_presenter.gizmo().has_value(), "layer update retains selection overlay");
  Check(!g_presenter.guides().empty(), "layer update retains align guides");

  g_frames = visible;
  MsgWndProc(nullptr, WM_VIDEO_FRAME, 0, 0);
  Check(g_frames == visible, "obsolete frame notification cannot clear the visible stack");

  g_pending_layers.clear();
  g_layers_configuring = true;
  CmdLayersCommit();
  Check(!g_frames, "empty layer stack clears the previous picture immediately");
  Check(!g_presenter.gizmo(), "empty layer stack clears the selection overlay");
  Check(g_presenter.guides().empty(), "empty layer stack clears align guides");
}

void TestRetainedDecoding(const std::wstring& path) {
  VideoEngine& engine = g_video;
  // No message pump: latest() is the same handoff consumed by the real host.
  engine.start(nullptr, WM_VIDEO_FRAME);
  std::vector<LayerSpec> layers{{VideoFrame::Kind::Video, path, 0, {}}, {VideoFrame::Kind::Video, path, 1, {}}};
  engine.setLayers(layers, 100);
  auto original = WaitForStack(engine, 100, 2);
  if (!original) return;
  Check(original->size() == 2, "duplicate media produces two independently retained layers");
  g_source_generation = 100;
  MsgWndProc(nullptr, WM_VIDEO_FRAME, 0, 0);
  Check(g_frames == original, "host accepts the initial complete stack");

  for (uint64_t generation = 101; generation < 121; ++generation) {
    layers[1].transform.x = (generation - 100) / 100.0;
    layers[1].transform.opacity = 0.5;
    engine.setLayers(layers, generation);
    auto updated = WaitForStack(engine, generation, 2);
    if (!updated) return;
    Check(updated->size() == 2, "dragging retains the complete layer stack");
    for (size_t i = 0; i < updated->size(); ++i) {
      Check((*updated)[i].texture == (*original)[i].texture,
            "transform-only update reuses the retained GPU texture");
      Check((*updated)[i].serial == (*original)[i].serial,
            "transform-only update does not create a new decoding completion");
    }
    Check(updated->back().transform.x == layers[1].transform.x &&
          updated->back().transform.opacity == 0.5, "drag updates placement and opacity");
    Check(original->back().transform.x == 0, "published stacks remain immutable");
    g_source_generation = generation;
    MsgWndProc(nullptr, WM_VIDEO_FRAME, 0, 0);
    Check(g_frames == updated, "host accepts transform changes on the same decoded frames");
  }

  engine.setLayers({}, 200);
  Check(!engine.latest(), "clearing invalidates completed decoding immediately");
  engine.setLayers(layers, 201);
  auto reopened = WaitForStack(engine, 201, 2);
  Check(reopened && reopened->size() == 2, "layers can be restored after clearing");
}
// Opaque stills keep the NV12 path; any transparency keeps premultiplied BGRA.
void TestStillFormats(const std::wstring& opaque, const std::wstring& translucent) {
  HwDecoder solid;
  Check(solid.open(opaque.c_str()) && solid.frame() &&
            solid.frame()->format == DXGI_FORMAT_NV12,
        "opaque still decodes to NV12");
  HwDecoder text;
  Check(text.open(translucent.c_str()) && text.frame() &&
            text.frame()->format == DXGI_FORMAT_B8G8R8A8_UNORM,
        "translucent still keeps alpha as BGRA");
  Check(text.frame() && text.frame()->width == 3 && text.frame()->height == 3,
        "BGRA stills keep odd dimensions");
}
}  // namespace

int wmain(int argc, wchar_t** argv) {
  if (argc != 3) return 2;
  TestPendingPresentation();
  TestRetainedDecoding(argv[1]);
  TestStillFormats(argv[1], argv[2]);
  g_frames.reset();
  g_video.shutdown();
  return failures ? 1 : 0;
}
