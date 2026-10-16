#pragma once

#include "decoder.h"
#include "transform/visual_transform.h"

#include <windows.h>
#include <wrl/client.h>
#include <memory>
#include <string>
#include <vector>

// Immutable handoff to the window thread. Retain both the texture and MF sample:
// holding only the texture would let the decoder recycle the underlying surface.
struct VideoFrame {
  // Decode identity stays stable when only placement/opacity changes.
  uint64_t serial = 0;
  uint64_t generation = 0;
  // Layer paths identity; frames are only shown while the request has the same sources.
  uint64_t structure = 0;
  // False while scrubbing: the closest frame decoded so far, not yet the target.
  bool exact = true;
  double target = 0;
  // Cold partial stacks can omit the requested top layer. An available bottom
  // layer must not report its aspect as that requested top source's aspect.
  bool topmost = true;
  std::shared_ptr<MfRuntime> runtime;
  DecoderFrame frame;
  Microsoft::WRL::ComPtr<ID3D11Texture2D> texture;
  Microsoft::WRL::ComPtr<IMFSample> sample;
  Microsoft::WRL::ComPtr<ID3D11Device> device;
  Microsoft::WRL::ComPtr<ID3D11DeviceContext> context;
  VisualTransform transform;
};

struct LayerSpec {
  std::wstring path;
  double time = 0;
  VisualTransform transform;
};

// At most kMaxLayers persistent stream workers and one immutable composed slot.
// Each owns its decoder on a COM thread, sharing a protected D3D device and a
// bounded aggregate history cache. Ready streams publish independently; missing
// cold streams are omitted until ready, with original stacking/placement retained.
// A source change only cancels that stream. Approximate stacks become exact only
// once every active stream has decoded the latest requested media time.
class VideoEngine {
 public:
  static constexpr size_t kMaxLayers = 4;

  VideoEngine();
  ~VideoEngine();
  VideoEngine(const VideoEngine&) = delete;
  VideoEngine& operator=(const VideoEngine&) = delete;

  void start(HWND window, UINT readyMessage);
  void setPath(std::wstring path, uint64_t generation = 0);
  void seek(double seconds);
  void setLayers(std::vector<LayerSpec> layers, uint64_t generation = 0);
  std::shared_ptr<const std::vector<VideoFrame>> latest() const;
  void shutdown();

 private:
  struct State;
  std::unique_ptr<State> state_;
};
