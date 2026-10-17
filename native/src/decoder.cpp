#include "decoder.h"

#include <mferror.h>
#include <wincodec.h>
#include <algorithm>
#include <chrono>
#include <cmath>
#include <iterator>
#include <cstring>
#include <vector>

#pragma comment(lib, "windowscodecs.lib")

namespace {

template <class T>
void SafeRelease(T** pp) {
  if (pp && *pp) {
    (*pp)->Release();
    *pp = nullptr;
  }
}

HRESULT SetPositionSeconds(IMFSourceReader* reader, double seconds) {
  PROPVARIANT var;
  PropVariantInit(&var);
  var.vt = VT_I8;
  var.hVal.QuadPart = static_cast<LONGLONG>(seconds * 10000000.0);
  // Seek near keyframe (Source Reader default for video)
  const HRESULT hr = reader->SetCurrentPosition(GUID_NULL, var);
  PropVariantClear(&var);
  return hr;
}

const wchar_t* PathExtension(const wchar_t* path) {
  if (!path || !*path) return L"";
  const wchar_t* last_dot = nullptr;
  for (const wchar_t* p = path; *p; ++p) {
    if (*p == L'.') last_dot = p;
    if (*p == L'/' || *p == L'\\') last_dot = nullptr;
  }
  return last_dot ? last_dot : L"";
}

bool EqualsIgnoreCase(const wchar_t* a, const wchar_t* b) {
  while (*a && *b) {
    const wchar_t ca = (*a >= L'A' && *a <= L'Z') ? (*a + (L'a' - L'A')) : *a;
    const wchar_t cb = (*b >= L'A' && *b <= L'Z') ? (*b + (L'a' - L'A')) : *b;
    if (ca != cb) return false;
    ++a;
    ++b;
  }
  return *a == *b;
}

void BgraToNv12(const uint8_t* bgra, UINT width, UINT height, UINT stride, std::vector<uint8_t>* out) {
  // Even dimensions required for NV12; odd edges are cropped.
  const UINT w = width & ~1u;
  const UINT h = height & ~1u;
  out->assign(static_cast<size_t>(w) * h * 3 / 2, 0);
  uint8_t* y_plane = out->data();
  uint8_t* uv_plane = y_plane + static_cast<size_t>(w) * h;
  for (UINT row = 0; row < h; ++row) {
    const uint8_t* src = bgra + static_cast<size_t>(row) * stride;
    uint8_t* y_row = y_plane + static_cast<size_t>(row) * w;
    for (UINT col = 0; col < w; ++col) {
      const float b = src[col * 4 + 0];
      const float g = src[col * 4 + 1];
      const float r = src[col * 4 + 2];
      // Full-range BT.709 luma.
      const float y = 0.2126f * r + 0.7152f * g + 0.0722f * b;
      y_row[col] = static_cast<uint8_t>(std::clamp(y + 0.5f, 0.f, 255.f));
    }
  }
  for (UINT row = 0; row < h; row += 2) {
    const uint8_t* src0 = bgra + static_cast<size_t>(row) * stride;
    const uint8_t* src1 = bgra + static_cast<size_t>(row + 1) * stride;
    uint8_t* uv_row = uv_plane + static_cast<size_t>(row / 2) * w;
    for (UINT col = 0; col < w; col += 2) {
      float b = 0, g = 0, r = 0;
      for (int dy = 0; dy < 2; ++dy) {
        const uint8_t* row_src = dy ? src1 : src0;
        for (int dx = 0; dx < 2; ++dx) {
          const UINT x = col + static_cast<UINT>(dx);
          b += row_src[x * 4 + 0];
          g += row_src[x * 4 + 1];
          r += row_src[x * 4 + 2];
        }
      }
      b *= 0.25f;
      g *= 0.25f;
      r *= 0.25f;
      const float y = 0.2126f * r + 0.7152f * g + 0.0722f * b;
      const float u = (b - y) / 1.8556f + 128.f;
      const float v = (r - y) / 1.5748f + 128.f;
      uv_row[col] = static_cast<uint8_t>(std::clamp(u + 0.5f, 0.f, 255.f));
      uv_row[col + 1] = static_cast<uint8_t>(std::clamp(v + 0.5f, 0.f, 255.f));
    }
  }
}

}  // namespace

bool HwDecoder::IsImagePath(const wchar_t* path) {
  const wchar_t* ext = PathExtension(path);
  return EqualsIgnoreCase(ext, L".png") || EqualsIgnoreCase(ext, L".jpg") ||
         EqualsIgnoreCase(ext, L".jpeg") || EqualsIgnoreCase(ext, L".webp") ||
         EqualsIgnoreCase(ext, L".bmp") || EqualsIgnoreCase(ext, L".gif") ||
         EqualsIgnoreCase(ext, L".tif") || EqualsIgnoreCase(ext, L".tiff");
}

HwDecoder::HwDecoder() = default;

std::shared_ptr<MfRuntime> MfRuntime::acquire() {
  if (FAILED(MFStartup(MF_VERSION))) return {};
  return std::shared_ptr<MfRuntime>(new MfRuntime());
}

MfRuntime::~MfRuntime() { MFShutdown(); }

HwDecoder::~HwDecoder() {
  close();
  SafeRelease(&dxgi_manager_);
  SafeRelease(&context_);
  SafeRelease(&device_);
}

bool HwDecoder::attachDevice(ID3D11Device* device, ID3D11DeviceContext* context,
                             IMFDXGIDeviceManager* manager, UINT reset_token) {
  if (!device || !context || !manager) return false;
  if (device_ == device && context_ == context && dxgi_manager_ == manager &&
      reset_token_ == reset_token) {
    return true;
  }
  close();
  SafeRelease(&dxgi_manager_);
  SafeRelease(&context_);
  SafeRelease(&device_);
  if (!mf_runtime_) {
    mf_runtime_ = MfRuntime::acquire();
    if (!mf_runtime_) return false;
  }
  device_ = device;
  context_ = context;
  dxgi_manager_ = manager;
  reset_token_ = reset_token;
  device_->AddRef();
  context_->AddRef();
  dxgi_manager_->AddRef();
  return true;
}

bool HwDecoder::EnsureDevice() {
  if (device_) return true;

  if (!mf_runtime_) {
    mf_runtime_ = MfRuntime::acquire();
    if (!mf_runtime_) return false;
  }

  D3D_FEATURE_LEVEL levels[] = {
      D3D_FEATURE_LEVEL_11_1,
      D3D_FEATURE_LEVEL_11_0,
      D3D_FEATURE_LEVEL_10_1,
      D3D_FEATURE_LEVEL_10_0,
  };
  D3D_FEATURE_LEVEL got = D3D_FEATURE_LEVEL_11_0;
  HRESULT hr = D3D11CreateDevice(
      nullptr, D3D_DRIVER_TYPE_HARDWARE, nullptr, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, levels,
      ARRAYSIZE(levels), D3D11_SDK_VERSION, &device_, &got, &context_);
  if (FAILED(hr)) {
    hr = D3D11CreateDevice(
        nullptr, D3D_DRIVER_TYPE_HARDWARE, nullptr, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, nullptr, 0,
        D3D11_SDK_VERSION, &device_, &got, &context_);
  }
  if (FAILED(hr) || !device_) return false;

  ID3D10Multithread* mt = nullptr;
  if (SUCCEEDED(device_->QueryInterface(IID_PPV_ARGS(&mt))) && mt) {
    mt->SetMultithreadProtected(TRUE);
    mt->Release();
  }

  hr = MFCreateDXGIDeviceManager(&reset_token_, &dxgi_manager_);
  if (FAILED(hr)) return false;
  hr = dxgi_manager_->ResetDevice(device_, reset_token_);
  return SUCCEEDED(hr);
}

bool HwDecoder::ConfigureVideoStream() {
  if (!reader_) return false;

  IMFMediaType* native = nullptr;
  HRESULT hr = reader_->GetNativeMediaType(
      static_cast<DWORD>(MF_SOURCE_READER_FIRST_VIDEO_STREAM), 0, &native);
  if (FAILED(hr) || !native) return false;

  GUID major = {};
  native->GetGUID(MF_MT_MAJOR_TYPE, &major);
  ReadVideoInfo(native);
  SafeRelease(&native);
  if (major != MFMediaType_Video) return false;

  IMFMediaType* out = nullptr;
  hr = MFCreateMediaType(&out);
  if (FAILED(hr)) return false;
  out->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video);
  // NV12 stays on GPU (DXGI surface)
  out->SetGUID(MF_MT_SUBTYPE, MFVideoFormat_NV12);
  hr = reader_->SetCurrentMediaType(
      static_cast<DWORD>(MF_SOURCE_READER_FIRST_VIDEO_STREAM), nullptr, out);
  SafeRelease(&out);
  if (FAILED(hr)) return false;

  IMFMediaType* current = nullptr;
  if (SUCCEEDED(reader_->GetCurrentMediaType(
          static_cast<DWORD>(MF_SOURCE_READER_FIRST_VIDEO_STREAM), &current))) {
    ReadVideoInfo(current);
    SafeRelease(&current);
  }

  hr = reader_->SetStreamSelection(static_cast<DWORD>(MF_SOURCE_READER_ALL_STREAMS), FALSE);
  if (FAILED(hr)) return false;
  hr = reader_->SetStreamSelection(
      static_cast<DWORD>(MF_SOURCE_READER_FIRST_VIDEO_STREAM), TRUE);
  return SUCCEEDED(hr);
}

void HwDecoder::ReadVideoInfo(IMFMediaType* type) {
  UINT32 a = 0, b = 0;
  if (SUCCEEDED(MFGetAttributeSize(type, MF_MT_FRAME_SIZE, &a, &b)) && a && b) {
    video_width_ = a;
    video_height_ = b;
    crop_x_ = crop_y_ = 0;
  }
  MFVideoArea area{};
  UINT32 size = 0;
  HRESULT aperture = type->GetBlob(MF_MT_MINIMUM_DISPLAY_APERTURE,
      reinterpret_cast<UINT8*>(&area), sizeof(area), &size);
  if (FAILED(aperture)) aperture = type->GetBlob(MF_MT_GEOMETRIC_APERTURE,
      reinterpret_cast<UINT8*>(&area), sizeof(area), &size);
  if (SUCCEEDED(aperture) && size == sizeof(area) && area.Area.cx > 0 && area.Area.cy > 0 &&
      static_cast<UINT32>(area.Area.cx) <= video_width_ && static_cast<UINT32>(area.Area.cy) <= video_height_) {
    video_width_ = area.Area.cx;
    video_height_ = area.Area.cy;
    crop_x_ = (std::max)(0.f, area.OffsetX.value + area.OffsetX.fract / 65536.f);
    crop_y_ = (std::max)(0.f, area.OffsetY.value + area.OffsetY.fract / 65536.f);
  }
  if (SUCCEEDED(MFGetAttributeRatio(type, MF_MT_FRAME_RATE, &a, &b)) && a && b) {
    frame_duration_ = static_cast<double>(b) / a;
  }
  if (SUCCEEDED(MFGetAttributeRatio(type, MF_MT_PIXEL_ASPECT_RATIO, &a, &b)) && a && b) {
    pixel_aspect_ratio_ = static_cast<float>(a) / b;
  }
  if (SUCCEEDED(type->GetUINT32(MF_MT_VIDEO_NOMINAL_RANGE, &a))) {
    full_range_ = a == MFNominalRange_0_255;
  }
  if (SUCCEEDED(type->GetUINT32(MF_MT_YUV_MATRIX, &a)) && a != MFVideoTransferMatrix_Unknown) {
    yuv_matrix_ = a;
    matrix_known_ = true;
  } else if (!matrix_known_ && video_width_ && video_height_) {
    yuv_matrix_ = video_height_ > 576 ? MFVideoTransferMatrix_BT709 : MFVideoTransferMatrix_BT601;
  }
}

void HwDecoder::ReleaseFrame() {
  SafeRelease(&frame_.texture);
  SafeRelease(&sample_);
  frame_ = DecoderFrame{};
  frame_from_cache_ = false;
}

void HwDecoder::ClearCache() {
  while (!cache_.empty()) EvictCachedFrame();
}

void HwDecoder::EvictCachedFrame() {
  const uint64_t bytes = cache_.front().bytes;
  SafeRelease(&cache_.front().frame.texture);
  cache_.pop_front();
  cache_bytes_ -= bytes;
  cache_budget_->release(bytes);
}

void HwDecoder::setCacheBudget(std::shared_ptr<DecoderCacheBudget> shared, uint64_t limit) {
  if (!shared) return;
  if (cache_budget_ != shared) {
    ClearCache();
    cache_budget_ = std::move(shared);
  }
  cache_limit_ = limit;
  while (!cache_.empty() && cache_bytes_ > cache_limit_) EvictCachedFrame();
}

void HwDecoder::CacheFrame() {
  // Cache independent GPU allocations, never decoder samples/surface leases.
  // Both limits bound memory for 4K and history for small videos.
  constexpr size_t frame_limit = 240;
  for (const auto& entry : cache_) {
    if (std::abs(entry.frame.timestamp - frame_.timestamp) < 1e-7) return;
  }
  D3D11_TEXTURE2D_DESC desc{};
  frame_.texture->GetDesc(&desc);
  const uint64_t bytes = static_cast<uint64_t>(desc.Width) * desc.Height *
      (desc.Format == DXGI_FORMAT_NV12 ? 3 : 6) / 2;
  if (bytes > cache_limit_ || (desc.Format != DXGI_FORMAT_NV12 && desc.Format != DXGI_FORMAT_P010 &&
                         desc.Format != DXGI_FORMAT_P016)) return;
  while (!cache_.empty() && (cache_bytes_ + bytes > cache_limit_ || cache_.size() >= frame_limit)) {
    EvictCachedFrame();
  }
  while (!cache_budget_->tryAcquire(bytes)) {
    if (cache_.empty()) return;
    EvictCachedFrame();
  }
  desc.ArraySize = 1;
  desc.MipLevels = 1;
  desc.Usage = D3D11_USAGE_DEFAULT;
  desc.BindFlags = D3D11_BIND_SHADER_RESOURCE;
  desc.CPUAccessFlags = 0;
  desc.MiscFlags = 0;
  ID3D11Texture2D* copy = nullptr;
  if (FAILED(device_->CreateTexture2D(&desc, nullptr, &copy))) {
    cache_budget_->release(bytes);
    return;
  }
  context_->CopySubresourceRegion(copy, 0, 0, 0, 0, frame_.texture, frame_.array_slice, nullptr);
  DecoderFrame cached = frame_;  // Preserve crop, padding, duration and colour metadata.
  cached.texture = copy;
  cached.array_slice = 0;
  cache_.push_back({cached, bytes});
  cache_bytes_ += bytes;
}

bool HwDecoder::FindCachedFrame(double seconds) {
  for (auto it = cache_.rbegin(); it != cache_.rend(); ++it) {
    if (seconds >= it->frame.timestamp - 1e-6 &&
        seconds < it->frame.timestamp + it->frame.duration - 1e-6) {
      DecoderFrame cached = it->frame;
      cached.texture->AddRef();
      ReleaseFrame();
      frame_ = cached;
      frame_from_cache_ = true;
      return true;
    }
  }
  return false;
}

bool HwDecoder::ReadOneFrame(const std::function<bool()>& cancelled) {
  if (!reader_ || !device_) return false;
  end_of_stream_ = false;

  for (int attempt = 0; attempt < 64; ++attempt) {
    if (cancelled && cancelled()) return false;
    DWORD stream = 0;
    DWORD flags = 0;
    LONGLONG ts = 0;
    IMFSample* sample = nullptr;
    HRESULT hr = reader_->ReadSample(
        static_cast<DWORD>(MF_SOURCE_READER_FIRST_VIDEO_STREAM), 0, &stream, &flags, &ts,
        &sample);
    if (FAILED(hr)) {
      SafeRelease(&sample);
      return false;
    }
    if (flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED) {
      ClearCache();
      IMFMediaType* type = nullptr;
      if (SUCCEEDED(reader_->GetCurrentMediaType(stream, &type))) {
        ReadVideoInfo(type);
        SafeRelease(&type);
      }
    }
    if ((flags & MF_SOURCE_READERF_ENDOFSTREAM) && !sample) {
      end_of_stream_ = true;
      SafeRelease(&sample);
      return false;
    }
    if (flags & MF_SOURCE_READERF_STREAMTICK) {
      SafeRelease(&sample);
      continue;
    }
    if (!sample) continue;

    IMFMediaBuffer* buffer = nullptr;
    DWORD count = 0;
    sample->GetBufferCount(&count);
    if (count > 0) sample->GetBufferByIndex(0, &buffer);
    if (!buffer) {
      SafeRelease(&sample);
      continue;
    }

    IMFDXGIBuffer* dxgi_buf = nullptr;
    hr = buffer->QueryInterface(IID_PPV_ARGS(&dxgi_buf));
    if (SUCCEEDED(hr) && dxgi_buf) {
      ID3D11Texture2D* tex = nullptr;
      hr = dxgi_buf->GetResource(IID_PPV_ARGS(&tex));
      UINT slice = 0;
      dxgi_buf->GetSubresourceIndex(&slice);
      if (SUCCEEDED(hr) && tex) {
        D3D11_TEXTURE2D_DESC desc{};
        tex->GetDesc(&desc);
        ReleaseFrame();
        frame_.texture = tex;
        frame_.array_slice = slice;
        frame_.width = static_cast<int>(video_width_ ? (std::min)(video_width_, desc.Width) : desc.Width);
        frame_.height = static_cast<int>(video_height_ ? (std::min)(video_height_, desc.Height) : desc.Height);
        frame_.format = desc.Format;
        frame_.timestamp = ts / 10000000.0;
        LONGLONG duration = 0;
        frame_.duration = SUCCEEDED(sample->GetSampleDuration(&duration)) && duration > 0
                              ? duration / 10000000.0 : frame_duration_;
        frame_.pixel_aspect_ratio = pixel_aspect_ratio_;
        frame_.crop_x = crop_x_;
        frame_.crop_y = crop_y_;
        frame_.full_range = full_range_;
        frame_.yuv_matrix = yuv_matrix_;
        sample_ = sample;
        CacheFrame();
        SafeRelease(&dxgi_buf);
        SafeRelease(&buffer);
        return true;
      }
      SafeRelease(&tex);
      SafeRelease(&dxgi_buf);
    }
    SafeRelease(&buffer);
    SafeRelease(&sample);
  }
  return false;
}

bool HwDecoder::OpenStill(const wchar_t* path) {
  if (!EnsureDevice()) return false;

  IWICImagingFactory* factory = nullptr;
  HRESULT hr = CoCreateInstance(
      CLSID_WICImagingFactory, nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&factory));
  if (FAILED(hr) || !factory) return false;

  IWICBitmapDecoder* decoder = nullptr;
  hr = factory->CreateDecoderFromFilename(
      path, nullptr, GENERIC_READ, WICDecodeMetadataCacheOnLoad, &decoder);
  if (FAILED(hr) || !decoder) {
    SafeRelease(&factory);
    return false;
  }

  IWICBitmapFrameDecode* frame = nullptr;
  hr = decoder->GetFrame(0, &frame);
  if (FAILED(hr) || !frame) {
    SafeRelease(&decoder);
    SafeRelease(&factory);
    return false;
  }

  IWICFormatConverter* converter = nullptr;
  hr = factory->CreateFormatConverter(&converter);
  if (FAILED(hr) || !converter) {
    SafeRelease(&frame);
    SafeRelease(&decoder);
    SafeRelease(&factory);
    return false;
  }

  hr = converter->Initialize(
      frame, GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, nullptr, 0.0,
      WICBitmapPaletteTypeCustom);
  if (FAILED(hr)) {
    SafeRelease(&converter);
    SafeRelease(&frame);
    SafeRelease(&decoder);
    SafeRelease(&factory);
    return false;
  }

  UINT width = 0, height = 0;
  hr = converter->GetSize(&width, &height);
  if (FAILED(hr) || width < 1 || height < 1) {
    SafeRelease(&converter);
    SafeRelease(&frame);
    SafeRelease(&decoder);
    SafeRelease(&factory);
    return false;
  }

  const UINT stride = width * 4;
  std::vector<uint8_t> bgra(static_cast<size_t>(stride) * height);
  hr = converter->CopyPixels(nullptr, stride, static_cast<UINT>(bgra.size()), bgra.data());
  SafeRelease(&converter);
  SafeRelease(&frame);
  SafeRelease(&decoder);
  SafeRelease(&factory);
  if (FAILED(hr)) return false;

  // A reveal atlas carries appearance times in its pixels: never convert it.
  const std::wstring lower = [&]() {
    std::wstring text(path);
    for (auto& c : text) c = static_cast<wchar_t>(towlower(c));
    return text;
  }();
  bool translucent = lower.size() >= 11 && lower.compare(lower.size() - 11, 11, L".reveal.png") == 0;
  for (size_t i = 3; !translucent && i < bgra.size(); i += 4) {
    if (bgra[i] != 255) translucent = true;
  }

  D3D11_TEXTURE2D_DESC desc{};
  desc.MipLevels = 1;
  desc.ArraySize = 1;
  desc.SampleDesc.Count = 1;
  desc.Usage = D3D11_USAGE_DEFAULT;
  desc.BindFlags = D3D11_BIND_SHADER_RESOURCE;
  D3D11_SUBRESOURCE_DATA init{};
  std::vector<uint8_t> nv12;
  if (translucent) {
    desc.Width = width;
    desc.Height = height;
    desc.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
    init.pSysMem = bgra.data();
    init.SysMemPitch = stride;
  } else {
    // Opaque: premultiplied equals straight, so the NV12 conversion is unchanged.
    BgraToNv12(bgra.data(), width, height, stride, &nv12);
    desc.Width = width & ~1u;
    desc.Height = height & ~1u;
    if (desc.Width < 2 || desc.Height < 2) return false;
    desc.Format = DXGI_FORMAT_NV12;
    init.pSysMem = nv12.data();
    init.SysMemPitch = desc.Width;
    init.SysMemSlicePitch = static_cast<UINT>(nv12.size());
  }
  const UINT even_w = desc.Width;
  const UINT even_h = desc.Height;

  ID3D11Texture2D* texture = nullptr;
  hr = device_->CreateTexture2D(&desc, &init, &texture);
  if (FAILED(hr) || !texture) return false;

  ReleaseFrame();
  frame_.texture = texture;
  frame_.array_slice = 0;
  frame_.width = static_cast<int>(even_w);
  frame_.height = static_cast<int>(even_h);
  frame_.format = desc.Format;
  frame_.timestamp = 0;
  // Long duration so any timeline seek stays "inside" this single frame.
  frame_.duration = 1.0e9;
  frame_.pixel_aspect_ratio = 1.f;
  frame_.crop_x = frame_.crop_y = 0;
  frame_.full_range = true;
  frame_.yuv_matrix = MFVideoTransferMatrix_BT709;
  video_width_ = even_w;
  video_height_ = even_h;
  frame_duration_ = frame_.duration;
  media_duration_ = frame_.duration;
  pixel_aspect_ratio_ = 1.f;
  full_range_ = true;
  yuv_matrix_ = MFVideoTransferMatrix_BT709;
  matrix_known_ = true;
  is_still_ = true;
  opened_ = true;
  frame_from_cache_ = false;
  return true;
}

bool HwDecoder::open(const wchar_t* path) {
  close();
  if (!path || !*path) return false;
  if (!EnsureDevice()) return false;

  if (IsImagePath(path)) {
    return OpenStill(path);
  }

  IMFAttributes* attrs = nullptr;
  HRESULT hr = MFCreateAttributes(&attrs, 3);
  if (FAILED(hr)) return false;
  attrs->SetUINT32(MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, TRUE);
  attrs->SetUINT32(MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, TRUE);
  attrs->SetUnknown(MF_SOURCE_READER_D3D_MANAGER, dxgi_manager_);

  hr = MFCreateSourceReaderFromURL(path, attrs, &reader_);
  SafeRelease(&attrs);
  if (FAILED(hr) || !reader_) return false;

  if (!ConfigureVideoStream()) {
    close();
    return false;
  }
  opened_ = true;
  is_still_ = false;
  PROPVARIANT duration;
  PropVariantInit(&duration);
  if (SUCCEEDED(reader_->GetPresentationAttribute(
          MF_SOURCE_READER_MEDIASOURCE, MF_PD_DURATION, &duration)) && duration.vt == VT_UI8) {
    media_duration_ = duration.uhVal.QuadPart / 10000000.0;
  }
  PropVariantClear(&duration);
  // The worker seeks directly to its latest target, avoiding an obsolete frame-zero decode.
  return true;
}

namespace {

// Without a learned keyframe spacing, follow a moving target at most this far
// before repositioning. Decoding 2 s forward costs about as much as a typical
// keyframe preroll, so a short-GOP source never decodes much more than needed.
constexpr double kDefaultForwardReach = 2.0;
// Landing points can skip keyframes, so a learned spacing may overestimate.
constexpr double kMaxForwardReach = 10.0;
// While chasing a moving target, hand back progress at least this often.
constexpr auto kChaseBudget = std::chrono::milliseconds(40);
constexpr size_t kMaxKeyframes = 4096;

}  // namespace

void HwDecoder::NoteKeyframe(double seconds) {
  if (!std::isfinite(seconds) || seconds < 0) return;
  // Timestamps are 100 ns ticks; treat values within half a millisecond as one keyframe.
  auto close_by = keyframes_.lower_bound(seconds - 5e-4);
  if (close_by != keyframes_.end() && *close_by <= seconds + 5e-4) return;
  if (keyframes_.size() >= kMaxKeyframes) return;
  const auto it = keyframes_.insert(seconds).first;
  // The minimum gap between neighbours never exceeds the real spacing.
  if (it != keyframes_.begin()) {
    const double gap = seconds - *std::prev(it);
    if (gap > 1e-3 && (keyframe_interval_ <= 0 || gap < keyframe_interval_)) keyframe_interval_ = gap;
  }
  if (const auto next = std::next(it); next != keyframes_.end()) {
    const double gap = *next - seconds;
    if (gap > 1e-3 && (keyframe_interval_ <= 0 || gap < keyframe_interval_)) keyframe_interval_ = gap;
  }
}

bool HwDecoder::CanDecodeForwardTo(double seconds) const {
  // A cached display does not move Source Reader; its cursor may be elsewhere.
  if (!frame_.texture || frame_from_cache_) return false;
  const double position = frame_.timestamp;
  if (seconds < position - 1e-6) return false;
  if (seconds < position + frame_.duration - 1e-6) return true;
  // Without a closer keyframe, decoding forward never costs more than the preroll
  // a reposition would need (it lands at or before the cursor's keyframe).
  const double reach = keyframe_interval_ > 0 ? (std::min)(keyframe_interval_, kMaxForwardReach)
                                              : kDefaultForwardReach;
  if (seconds - position > reach) return false;
  // Repositioning would land on a known keyframe after the cursor: that is closer.
  const auto next = keyframes_.upper_bound(position + 5e-4);
  return next == keyframes_.end() || *next > seconds + 1e-6;
}

bool HwDecoder::Reposition(double seconds, const std::function<bool()>& cancelled) {
  if (FAILED(SetPositionSeconds(reader_, seconds))) return false;
  ReleaseFrame();
  if (cancelled && cancelled()) return false;
  if (ReadOneFrame(cancelled)) {
    // SetCurrentPosition lands on the keyframe at or before the target.
    NoteKeyframe(frame_.timestamp);
    return true;
  }
  if ((cancelled && cancelled()) || !end_of_stream_) return false;
  // Container duration can include audio after the final video frame. Seek back
  // until a video sample is available, then advance to the final valid frame.
  double step = (std::max)(frame_duration_, 0.5);
  double position = seconds;
  while (!frame_.texture && position > 0) {
    if (cancelled && cancelled()) return false;
    position = (std::max)(0.0, seconds - step);
    if (FAILED(SetPositionSeconds(reader_, position))) return false;
    if (ReadOneFrame(cancelled)) {
      NoteKeyframe(frame_.timestamp);
      break;
    }
    if (cancelled && cancelled()) return false;
    if (!end_of_stream_) return false;
    step *= 2;
  }
  return frame_.texture != nullptr;
}

SeekResult HwDecoder::seek(double seconds, const SeekControl& control, double* served) {
  const auto cancelled = [&]() { return control.cancelled && control.cancelled(); };
  const auto clamp = [&](double value) {
    value = (std::max)(0.0, value);
    if (media_duration_ > 0) value = (std::min)(value, (std::max)(0.0, media_duration_ - 1e-6));
    return value;
  };
  if (served) *served = seconds;
  if (!opened_ || !std::isfinite(seconds)) return SeekResult::Failed;
  // Stills always show frame 0; ignore scrub time and never re-decode.
  if (is_still_) {
    if (cancelled()) return SeekResult::Abandoned;
    return frame_.texture ? SeekResult::Exact : SeekResult::Failed;
  }
  if (!reader_) return SeekResult::Failed;
  if (cancelled()) return SeekResult::Abandoned;
  // The request may already be stale when this layer's turn comes.
  if (control.newer) {
    if (const auto next = control.newer(); next && std::isfinite(*next)) seconds = *next;
  }
  // `served` reports the requested time (before clamping) so callers can compare
  // it with their own request; `target` is the clamped time actually decoded.
  double requested = seconds;
  double target = clamp(requested);
  if (served) *served = requested;
  const auto started = std::chrono::steady_clock::now();
  bool followed = false;
  const auto superseded = [&]() { return frame_.texture ? SeekResult::Partial : SeekResult::Abandoned; };

  if ((frame_from_cache_ || !frame_.texture || target < frame_.timestamp - 1e-6) &&
      FindCachedFrame(target)) {
    return SeekResult::Exact;
  }
  if (end_of_stream_ && !frame_from_cache_ && frame_.texture && target >= frame_.timestamp) {
    return SeekResult::Exact;
  }
  // Playback and continuous forward scrubbing consume the next samples without
  // seeking again; anything else repositions to the keyframe before the target.
  if (!CanDecodeForwardTo(target)) {
    if (!Reposition(target, cancelled)) {
      return cancelled() ? SeekResult::Abandoned : SeekResult::Failed;
    }
  }
  // Decode up to the frame containing the target, following newer targets of the
  // same source while they stay ahead of the decoder.
  while (target >= frame_.timestamp + frame_.duration - 1e-6) {
    if (cancelled()) return SeekResult::Abandoned;
    if (control.newer) {
      if (const auto next = control.newer(); next && std::isfinite(*next) && *next != requested) {
        followed = true;
        requested = *next;
        if (served) *served = requested;
        const double candidate = clamp(requested);
        if (!CanDecodeForwardTo(candidate)) return superseded();
        target = candidate;
        continue;
      }
      if (followed && std::chrono::steady_clock::now() - started >= kChaseBudget) {
        return superseded();
      }
    }
    if (!ReadOneFrame(cancelled)) {
      if (cancelled()) return SeekResult::Abandoned;
      return end_of_stream_ && frame_.texture ? SeekResult::Exact : SeekResult::Failed;
    }
  }
  if (cancelled()) return SeekResult::Abandoned;
  return frame_.texture ? SeekResult::Exact : SeekResult::Failed;
}

void HwDecoder::close() {
  ReleaseFrame();
  ClearCache();
  SafeRelease(&reader_);
  opened_ = false;
  is_still_ = false;
  video_width_ = video_height_ = 0;
  frame_duration_ = 1.0 / 30.0;
  media_duration_ = 0;
  pixel_aspect_ratio_ = 1.f;
  crop_x_ = crop_y_ = 0;
  full_range_ = false;
  yuv_matrix_ = MFVideoTransferMatrix_BT709;
  matrix_known_ = false;
  end_of_stream_ = false;
  keyframes_.clear();
  keyframe_interval_ = 0;
}
