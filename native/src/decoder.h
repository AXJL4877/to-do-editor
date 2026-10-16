#pragma once

#include "decoder_cache_budget.h"

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <d3d11.h>
#include <dxgi1_2.h>
#include <mfapi.h>
#include <mfidl.h>
#include <mfreadwrite.h>
#include <cstdint>
#include <functional>
#include <deque>
#include <optional>
#include <set>
#include <memory>

// MF samples can outlive their decoder through an immutable published frame.
// Keep Media Foundation started until the last such surface lease is released.
class MfRuntime {
 public:
  static std::shared_ptr<MfRuntime> acquire();
  ~MfRuntime();

 private:
  MfRuntime() = default;
};

struct DecoderFrame {
  ID3D11Texture2D* texture = nullptr;  // GPU surface, no CPU readback
  UINT array_slice = 0;
  int width = 0;
  int height = 0;
  DXGI_FORMAT format = DXGI_FORMAT_UNKNOWN;
  double timestamp = 0;
  double duration = 1.0 / 30.0;
  float pixel_aspect_ratio = 1.f;
  float crop_x = 0;
  float crop_y = 0;
  bool full_range = false;
  UINT32 yuv_matrix = MFVideoTransferMatrix_BT709;
};

// How a seek may be interrupted. `cancelled` abandons the request (source
// changed, shutdown). `newer` reports a later target for the same source while
// one is pending; the decoder follows it when decoding forward is cheaper than
// repositioning, otherwise it hands back its current frame so the caller can
// publish progress instead of discarding the work.
struct SeekControl {
  std::function<bool()> cancelled;
  std::function<std::optional<double>()> newer;
};

enum class SeekResult {
  Exact,      // frame contains the served target
  Partial,    // superseded; frame() is the closest frame decoded so far
  Abandoned,  // cancelled or superseded before any frame was available
  Failed,     // decoding error
};

// MP4 hardware decode / still-image WIC decode -> D3D11 texture.
// Opaque stills are converted to NV12 like video. Stills with any transparent
// pixel stay premultiplied BGRA (DXGI_FORMAT_B8G8R8A8_UNORM) so they composite.
// Owned by one COM-initialized thread.
class HwDecoder {
 public:
  HwDecoder();
  ~HwDecoder();

  HwDecoder(const HwDecoder&) = delete;
  HwDecoder& operator=(const HwDecoder&) = delete;

  static bool IsImagePath(const wchar_t* path);

  bool open(const wchar_t* path);
  // Create the protected device without opening a source. The scheduling thread
  // retains it until all stream workers and their COM-owned decoders have exited.
  bool prepareDevice() { return EnsureDevice(); }
  /// Borrow an existing D3D/MF device so multiple decoders can composite together.
  bool attachDevice(ID3D11Device* device, ID3D11DeviceContext* context,
                    IMFDXGIDeviceManager* manager, UINT reset_token);
  /// Decode the frame containing `seconds`. `served` receives the target that the
  /// result refers to, which differs from `seconds` after following `newer`.
  SeekResult seek(double seconds, const SeekControl& control = {}, double* served = nullptr);
  void close();
  void setCacheBudget(std::shared_ptr<DecoderCacheBudget> shared, uint64_t limit);

  ID3D11Device* device() const { return device_; }
  ID3D11DeviceContext* context() const { return context_; }
  IMFDXGIDeviceManager* dxgiManager() const { return dxgi_manager_; }
  UINT resetToken() const { return reset_token_; }
  const DecoderFrame* frame() const { return frame_.texture ? &frame_ : nullptr; }
  IMFSample* sample() const { return sample_; }
  bool isStill() const { return is_still_; }
  const std::shared_ptr<MfRuntime>& runtime() const { return mf_runtime_; }

 private:
  bool EnsureDevice();
  bool ConfigureVideoStream();
  bool OpenStill(const wchar_t* path);
  bool ReadOneFrame(const std::function<bool()>& cancelled);
  bool Reposition(double seconds, const std::function<bool()>& cancelled);
  bool CanDecodeForwardTo(double seconds) const;
  void NoteKeyframe(double seconds);
  void ReadVideoInfo(IMFMediaType* type);
  void ReleaseFrame();
  void CacheFrame();
  void ClearCache();
  void EvictCachedFrame();
  bool FindCachedFrame(double seconds);
  struct CachedFrame { DecoderFrame frame; uint64_t bytes; };
  std::deque<CachedFrame> cache_;
  uint64_t cache_bytes_ = 0;
  uint64_t cache_limit_ = 256ull * 1024 * 1024;
  std::shared_ptr<DecoderCacheBudget> cache_budget_ =
      std::make_shared<DecoderCacheBudget>(256ull * 1024 * 1024);
  bool frame_from_cache_ = false;

  ID3D11Device* device_ = nullptr;
  ID3D11DeviceContext* context_ = nullptr;
  IMFDXGIDeviceManager* dxgi_manager_ = nullptr;
  UINT reset_token_ = 0;
  IMFSourceReader* reader_ = nullptr;
  IMFSample* sample_ = nullptr;  // Keep the decoder's surface lease until the frame is replaced.
  DecoderFrame frame_{};
  UINT32 video_width_ = 0;
  UINT32 video_height_ = 0;
  double frame_duration_ = 1.0 / 30.0;
  double media_duration_ = 0;
  float pixel_aspect_ratio_ = 1.f;
  float crop_x_ = 0;
  float crop_y_ = 0;
  bool full_range_ = false;
  UINT32 yuv_matrix_ = MFVideoTransferMatrix_BT709;
  bool matrix_known_ = false;
  bool end_of_stream_ = false;
  bool opened_ = false;
  bool is_still_ = false;
  std::shared_ptr<MfRuntime> mf_runtime_;
  // Keyframe times observed where repositioning landed (decoded output samples
  // are all flagged as clean points, so they cannot identify keyframes), and the
  // smallest spacing between neighbours (0 = unknown).
  std::set<double> keyframes_;
  double keyframe_interval_ = 0;
};
