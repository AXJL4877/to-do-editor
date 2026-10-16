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
  double target = 0;
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

// One decoder worker with an overwritable layer stack and one completed stack slot.
// Decoders are pooled by path on a shared D3D device (max MAX_PREVIEW_LAYERS).
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
