#include "presenter.h"
#include "transform/align_guides_shader.h"
#include "transform/visual_transform_shader.h"
#include "transform/transform_gizmo_shader.h"
#include "transform/frame_display_geometry.h"
#include "viewport/preview_viewport_shader.h"

#include <algorithm>
#include <cstring>
#include <cstdio>
#include <d3d11_1.h>
#include <d3dcompiler.h>

#pragma comment(lib, "d3dcompiler.lib")

namespace {

template <class T>
void SafeRelease(T** pp) {
  if (pp && *pp) {
    (*pp)->Release();
    *pp = nullptr;
  }
}

bool CompilePs(ID3D11Device* device, const char* src, ID3D11PixelShader** out) {
  ID3DBlob* blob = nullptr;
  ID3DBlob* err = nullptr;
  HRESULT hr = D3DCompile(src, strlen(src), nullptr, nullptr, nullptr, "PSMain", "ps_4_0", 0, 0, &blob, &err);
  if (FAILED(hr) && err) fprintf(stderr, "[preview] pixel shader: %.*s\n",
                               static_cast<int>(err->GetBufferSize()), static_cast<const char*>(err->GetBufferPointer()));
  SafeRelease(&err);
  if (FAILED(hr) || !blob) return false;
  hr = device->CreatePixelShader(blob->GetBufferPointer(), blob->GetBufferSize(), nullptr, out);
  SafeRelease(&blob);
  return SUCCEEDED(hr);
}

bool CreateNv12Srvs(
    ID3D11Device* device, ID3D11Texture2D* tex,
    ID3D11ShaderResourceView** out_y, ID3D11ShaderResourceView** out_uv) {
  if (!device || !tex || !out_y || !out_uv) return false;
  *out_y = nullptr;
  *out_uv = nullptr;

  D3D11_SHADER_RESOURCE_VIEW_DESC yd{};
  yd.Format = DXGI_FORMAT_R8_UNORM;
  D3D11_SHADER_RESOURCE_VIEW_DESC uvd{};
  uvd.Format = DXGI_FORMAT_R8G8_UNORM;

  yd.ViewDimension = D3D11_SRV_DIMENSION_TEXTURE2D;
  yd.Texture2D.MostDetailedMip = 0;
  yd.Texture2D.MipLevels = 1;
  uvd.ViewDimension = D3D11_SRV_DIMENSION_TEXTURE2D;
  uvd.Texture2D.MostDetailedMip = 0;
  uvd.Texture2D.MipLevels = 1;
  if (FAILED(device->CreateShaderResourceView(tex, &yd, out_y))) return false;
  if (FAILED(device->CreateShaderResourceView(tex, &uvd, out_uv))) {
    SafeRelease(out_y);
    return false;
  }
  return true;
}

}  // namespace

GpuPresenter::GpuPresenter() = default;

GpuPresenter::~GpuPresenter() {
  ReleaseSizeDependent();
  ReleaseSamplingTexture();
  ReleaseStillTexture();
  ReleasePipeline();
  SafeRelease(&swap_);
  if (context_) context_->Flush();
  SafeRelease(&context_);
  SafeRelease(&device_);
}

void GpuPresenter::ReleasePipeline() {
  SafeRelease(&blend_);
  SafeRelease(&rast_);
  SafeRelease(&sampler_);
  SafeRelease(&ps_2d_);
  SafeRelease(&ps_still_);
  SafeRelease(&video_params_);
  SafeRelease(&ps_gizmo_);
  SafeRelease(&gizmo_params_);
  SafeRelease(&ps_guides_);
  SafeRelease(&guides_params_);
  SafeRelease(&ps_viewport_);
  SafeRelease(&viewport_params_);
  SafeRelease(&vs_);
}

void GpuPresenter::ReleaseSizeDependent() {
  if (context_) context_->OMSetRenderTargets(0, nullptr, nullptr);
  SafeRelease(&rtv_);
}

void GpuPresenter::ReleaseSamplingTexture() {
  if (context_) {
    ID3D11ShaderResourceView* empty[2] = {nullptr, nullptr};
    context_->PSSetShaderResources(0, 2, empty);
  }
  SafeRelease(&srv_y_);
  SafeRelease(&srv_uv_);
  SafeRelease(&sampling_texture_);
}

void GpuPresenter::ReleaseStillTexture() {
  if (context_) {
    ID3D11ShaderResourceView* empty = nullptr;
    context_->PSSetShaderResources(0, 1, &empty);
  }
  SafeRelease(&srv_still_);
  SafeRelease(&still_texture_);
}

bool GpuPresenter::bind(HWND hwnd, ID3D11Device* device, ID3D11DeviceContext* context) {
  if (hwnd_ == hwnd && device_ == device && context_ == context) return EnsurePipeline();
  ReleaseSizeDependent();
  SafeRelease(&swap_);
  if (context_) context_->Flush();  // Finish deferred destruction before replacing a flip swap chain.
  if (device_ != device || context_ != context) {
    ReleaseSamplingTexture();
    ReleaseStillTexture();
    ReleasePipeline();
    if (context_) {
      context_->ClearState();
      context_->Flush();
    }
    SafeRelease(&context_);
    SafeRelease(&device_);
    device_ = device;
    context_ = context;
    if (device_) device_->AddRef();
    if (context_) context_->AddRef();
  }
  hwnd_ = hwnd;
  width_ = height_ = 0;
  if (!device_ || !context_ || !hwnd_) return false;
  return EnsurePipeline();
}

bool GpuPresenter::EnsurePipeline() {
  if (vs_ && ps_2d_ && ps_still_ && sampler_ && rast_ && blend_ && video_params_ && ps_gizmo_ && gizmo_params_ &&
      ps_guides_ && guides_params_ && ps_viewport_ && viewport_params_) return true;
  if (!device_) return false;
  ReleasePipeline();

  ID3DBlob* vs_blob = nullptr;
  ID3DBlob* err = nullptr;
  HRESULT hr = D3DCompile(
      kVisualVertexShader, strlen(kVisualVertexShader), nullptr, nullptr, nullptr, "VSMain", "vs_4_0", 0, 0, &vs_blob, &err);
  SafeRelease(&err);
  if (FAILED(hr) || !vs_blob) return false;
  hr = device_->CreateVertexShader(vs_blob->GetBufferPointer(), vs_blob->GetBufferSize(), nullptr, &vs_);
  SafeRelease(&vs_blob);
  if (FAILED(hr)) return false;

  if (!CompilePs(device_, kVideoPixelShader, &ps_2d_)) return false;
  if (!CompilePs(device_, kStillPixelShader, &ps_still_)) return false;
  if (!CompilePs(device_, kGizmoPixelShader, &ps_gizmo_)) return false;
  if (!CompilePs(device_, kAlignGuidesPixelShader, &ps_guides_)) return false;
  if (!CompilePs(device_, kViewportPixelShader, &ps_viewport_)) return false;

  D3D11_SAMPLER_DESC sd{};
  sd.Filter = D3D11_FILTER_MIN_MAG_MIP_LINEAR;
  sd.AddressU = D3D11_TEXTURE_ADDRESS_CLAMP;
  sd.AddressV = D3D11_TEXTURE_ADDRESS_CLAMP;
  sd.AddressW = D3D11_TEXTURE_ADDRESS_CLAMP;
  sd.MaxLOD = D3D11_FLOAT32_MAX;
  hr = device_->CreateSamplerState(&sd, &sampler_);
  if (FAILED(hr)) return false;

  D3D11_RASTERIZER_DESC rd{};
  rd.FillMode = D3D11_FILL_SOLID;
  rd.CullMode = D3D11_CULL_NONE;
  rd.DepthClipEnable = TRUE;
  hr = device_->CreateRasterizerState(&rd, &rast_);
  if (FAILED(hr)) return false;

  D3D11_BLEND_DESC bd{};
  bd.RenderTarget[0].BlendEnable = TRUE;
  bd.RenderTarget[0].SrcBlend = D3D11_BLEND_ONE;
  bd.RenderTarget[0].DestBlend = D3D11_BLEND_INV_SRC_ALPHA;
  bd.RenderTarget[0].BlendOp = D3D11_BLEND_OP_ADD;
  bd.RenderTarget[0].SrcBlendAlpha = D3D11_BLEND_ONE;
  bd.RenderTarget[0].DestBlendAlpha = D3D11_BLEND_INV_SRC_ALPHA;
  bd.RenderTarget[0].BlendOpAlpha = D3D11_BLEND_OP_ADD;
  bd.RenderTarget[0].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL;
  hr = device_->CreateBlendState(&bd, &blend_);
  if (FAILED(hr)) return false;
  D3D11_BUFFER_DESC cb{};
  cb.ByteWidth = 6 * 4 * sizeof(float);
  cb.Usage = D3D11_USAGE_DEFAULT;
  cb.BindFlags = D3D11_BIND_CONSTANT_BUFFER;
  if (FAILED(device_->CreateBuffer(&cb, nullptr, &video_params_))) return false;
  cb.ByteWidth = 3 * 4 * sizeof(float);
  if (FAILED(device_->CreateBuffer(&cb, nullptr, &gizmo_params_))) return false;
  cb.ByteWidth = 3 * 4 * sizeof(float);
  if (FAILED(device_->CreateBuffer(&cb, nullptr, &guides_params_))) return false;
  cb.ByteWidth = 2 * 4 * sizeof(float);
  return SUCCEEDED(device_->CreateBuffer(&cb, nullptr, &viewport_params_));
}

bool GpuPresenter::EnsureSamplingTexture(const D3D11_TEXTURE2D_DESC& source) {
  // Video and stills both arrive as NV12 (stills are WIC→BGRA→NV12 in the decoder).
  if (source.Format != DXGI_FORMAT_NV12) return false;
  if (sampling_texture_) {
    D3D11_TEXTURE2D_DESC current{};
    sampling_texture_->GetDesc(&current);
    if (current.Width == source.Width && current.Height == source.Height &&
        current.Format == source.Format && srv_y_ && srv_uv_) return true;
  }
  ReleaseSamplingTexture();
  D3D11_TEXTURE2D_DESC desc{};
  desc.Width = source.Width;
  desc.Height = source.Height;
  desc.MipLevels = desc.ArraySize = 1;
  desc.Format = source.Format;
  desc.SampleDesc.Count = 1;
  desc.Usage = D3D11_USAGE_DEFAULT;
  desc.BindFlags = D3D11_BIND_SHADER_RESOURCE;
  if (FAILED(device_->CreateTexture2D(&desc, nullptr, &sampling_texture_))) return false;
  if (CreateNv12Srvs(device_, sampling_texture_, &srv_y_, &srv_uv_)) return true;
  ReleaseSamplingTexture();
  return false;
}

bool GpuPresenter::EnsureStillTexture(const D3D11_TEXTURE2D_DESC& source) {
  if (source.Format != DXGI_FORMAT_B8G8R8A8_UNORM) return false;
  if (still_texture_) {
    D3D11_TEXTURE2D_DESC current{};
    still_texture_->GetDesc(&current);
    if (current.Width == source.Width && current.Height == source.Height && srv_still_) return true;
  }
  ReleaseStillTexture();
  D3D11_TEXTURE2D_DESC desc{};
  desc.Width = source.Width;
  desc.Height = source.Height;
  desc.MipLevels = desc.ArraySize = 1;
  desc.Format = source.Format;
  desc.SampleDesc.Count = 1;
  desc.Usage = D3D11_USAGE_DEFAULT;
  desc.BindFlags = D3D11_BIND_SHADER_RESOURCE;
  if (FAILED(device_->CreateTexture2D(&desc, nullptr, &still_texture_))) return false;
  if (SUCCEEDED(device_->CreateShaderResourceView(still_texture_, nullptr, &srv_still_))) return true;
  ReleaseStillTexture();
  return false;
}

bool GpuPresenter::EnsureRtv() {
  if (rtv_) return true;
  if (!swap_ || !device_) return false;
  ID3D11Texture2D* back = nullptr;
  HRESULT hr = swap_->GetBuffer(0, IID_PPV_ARGS(&back));
  if (FAILED(hr) || !back) return false;
  hr = device_->CreateRenderTargetView(back, nullptr, &rtv_);
  SafeRelease(&back);
  return SUCCEEDED(hr) && rtv_;
}

bool GpuPresenter::EnsureSwapChain(int width, int height) {
  if (!device_ || !hwnd_ || width < 1 || height < 1) return false;

  if (swap_ && width_ == width && height_ == height) return EnsureRtv();

  if (swap_) {
    // CmdSize / WM_SIZE: drop RTV then ResizeBuffers
    ReleaseSizeDependent();
    HRESULT hr = swap_->ResizeBuffers(0, width, height, DXGI_FORMAT_UNKNOWN, 0);
    if (SUCCEEDED(hr)) {
      width_ = width;
      height_ = height;
      return EnsureRtv();
    }
    return false;
  }

  IDXGIDevice* dxgi_dev = nullptr;
  HRESULT hr = device_->QueryInterface(IID_PPV_ARGS(&dxgi_dev));
  if (FAILED(hr)) return false;
  IDXGIAdapter* adapter = nullptr;
  hr = dxgi_dev->GetAdapter(&adapter);
  SafeRelease(&dxgi_dev);
  if (FAILED(hr)) return false;
  IDXGIFactory2* factory = nullptr;
  hr = adapter->GetParent(IID_PPV_ARGS(&factory));
  SafeRelease(&adapter);
  if (FAILED(hr)) return false;

  DXGI_SWAP_CHAIN_DESC1 desc{};
  desc.Width = width;
  desc.Height = height;
  desc.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
  desc.SampleDesc.Count = 1;
  desc.BufferUsage = DXGI_USAGE_RENDER_TARGET_OUTPUT;
  desc.BufferCount = 2;
  desc.Scaling = DXGI_SCALING_STRETCH;
  desc.SwapEffect = DXGI_SWAP_EFFECT_FLIP_DISCARD;
  desc.AlphaMode = DXGI_ALPHA_MODE_IGNORE;

  hr = factory->CreateSwapChainForHwnd(device_, hwnd_, &desc, nullptr, nullptr, &swap_);
  if (SUCCEEDED(hr) && swap_) {
    factory->MakeWindowAssociation(hwnd_, DXGI_MWA_NO_ALT_ENTER);
  }
  SafeRelease(&factory);
  if (FAILED(hr) || !swap_) return false;

  width_ = width;
  height_ = height;
  return EnsureRtv();
}

void GpuPresenter::resize(int width, int height) {
  if (width < 1 || height < 1) return;
  EnsureSwapChain(width, height);
}

void GpuPresenter::clear() {
  presentLayers(nullptr, nullptr, 0);
}

bool GpuPresenter::setTransform(const VisualTransform& transform) {
  if (!transform.valid()) return false;
  transform_ = transform;
  return true;
}

bool GpuPresenter::setGizmo(const std::optional<TransformGizmo>& gizmo) {
  if (gizmo && !gizmo->valid()) return false;
  gizmo_ = gizmo;
  return true;
}

bool GpuPresenter::setGuides(std::vector<AlignGuide> guides) {
  if (guides.size() > kMaxAlignGuides) return false;
  for (const auto& guide : guides) {
    if (!guide.valid()) return false;
  }
  guides_ = std::move(guides);
  return true;
}

bool GpuPresenter::setViewport(const PreviewViewport& viewport) {
  if (!viewport.valid()) return false;
  viewport_ = viewport;
  return true;
}

void GpuPresenter::DrawViewport(bool overlay) {
  if (!viewport_.enabled) return;
  const float dpi = static_cast<float>(GetDpiForWindow(hwnd_)) / 96.f;
  const float params[8] = {
      FiniteVisualFloat(viewport_.left), FiniteVisualFloat(viewport_.top),
      FiniteVisualFloat(viewport_.width), FiniteVisualFloat(viewport_.height),
      width_ / dpi, height_ / dpi, overlay ? 1.f : 0.f, viewport_.show_outside ? 1.f : 0.f,
  };
  context_->UpdateSubresource(viewport_params_, 0, nullptr, params, 0, 0);
  context_->PSSetShader(ps_viewport_, nullptr, 0);
  context_->PSSetConstantBuffers(0, 1, &viewport_params_);
  context_->Draw(3, 0);
  context_->PSSetShader(ps_2d_, nullptr, 0);
  context_->PSSetConstantBuffers(0, 1, &video_params_);
}

void GpuPresenter::DrawGizmo() {
  if (!gizmo_) return;
  const float dpi_scale = static_cast<float>(GetDpiForWindow(hwnd_)) / 96.f;
  const float params[12] = {
      FiniteVisualFloat(gizmo_->left), FiniteVisualFloat(gizmo_->top),
      FiniteVisualFloat(gizmo_->right), FiniteVisualFloat(gizmo_->bottom),
      static_cast<float>(width_), static_cast<float>(height_), 0.75f * dpi_scale, 4.f * dpi_scale,
      gizmo_->editable ? 1.f : 0.f, 0.f, 0.f, 0.f,
  };
  context_->UpdateSubresource(gizmo_params_, 0, nullptr, params, 0, 0);
  context_->PSSetShader(ps_gizmo_, nullptr, 0);
  context_->PSSetConstantBuffers(0, 1, &gizmo_params_);
  context_->Draw(3, 0);
}

void GpuPresenter::DrawGuides() {
  if (guides_.empty()) return;
  const float dpi_scale = static_cast<float>(GetDpiForWindow(hwnd_)) / 96.f;
  float positions[4] = {};
  float axes[4] = {};
  const size_t count = (std::min)(guides_.size(), kMaxAlignGuides);
  for (size_t i = 0; i < count; ++i) {
    positions[i] = FiniteVisualFloat(guides_[i].position);
    axes[i] = guides_[i].vertical ? 0.f : 1.f;
  }
  const float params[12] = {
      positions[0], positions[1], positions[2], positions[3],
      axes[0], axes[1], axes[2], axes[3],
      static_cast<float>(width_), static_cast<float>(height_), 1.0f * dpi_scale,
      static_cast<float>(count),
  };
  context_->UpdateSubresource(guides_params_, 0, nullptr, params, 0, 0);
  context_->PSSetShader(ps_guides_, nullptr, 0);
  context_->PSSetConstantBuffers(0, 1, &guides_params_);
  context_->Draw(3, 0);
}

bool GpuPresenter::DrawLayer(const DecoderFrame& frame, const VisualTransform& transform) {
  if (!frame.texture || !context_ || !rtv_) return false;
  D3D11_TEXTURE2D_DESC source{};
  frame.texture->GetDesc(&source);
  const bool still = source.Format == DXGI_FORMAT_B8G8R8A8_UNORM;
  if (!(still ? EnsureStillTexture(source) : EnsureSamplingTexture(source)) || frame.width < 1 ||
      frame.height < 1 || frame.array_slice >= source.ArraySize * source.MipLevels) return false;
  context_->CopySubresourceRegion(still ? still_texture_ : sampling_texture_, 0, 0, 0, 0,
                                  frame.texture, frame.array_slice, nullptr);

  const auto geometry = DecodedFrameDisplayGeometry(frame, source.Width, source.Height);
  if (!geometry) return false;
  const float crop_x = geometry->crop_x, crop_y = geometry->crop_y;
  const float visible_w = geometry->visible_width, visible_h = geometry->visible_height;
  const VisualUvMap visual_map = MapViewportVisual(
      FitVisualTransform(width_ * viewport_.width, height_ * viewport_.height,
                         geometry->display_width, visible_h, transform), viewport_);

  float kr = 0.2126f, kb = 0.0722f;
  if (frame.yuv_matrix == MFVideoTransferMatrix_BT601) {
    kr = 0.299f;
    kb = 0.114f;
  } else if (frame.yuv_matrix == MFVideoTransferMatrix_SMPTE240M) {
    kr = 0.212f;
    kb = 0.087f;
  } else if (frame.yuv_matrix == MFVideoTransferMatrix_BT2020_10 ||
             frame.yuv_matrix == MFVideoTransferMatrix_BT2020_12) {
    kr = 0.2627f;
    kb = 0.0593f;
  }
  const float kg = 1.f - kr - kb;
  const float params[24] = {
      visible_w / source.Width, visible_h / source.Height, 0.5f / source.Width, 0.5f / source.Height,
      crop_x / source.Width, crop_y / source.Height, 0.f, 0.f,
      frame.full_range ? 0.f : 16.f / 255.f, frame.full_range ? 1.f : 255.f / 219.f,
      128.f / 255.f, frame.full_range ? 1.f : 255.f / 224.f,
      2.f * (1.f - kr), 2.f * kb * (1.f - kb) / kg,
      2.f * kr * (1.f - kr) / kg, 2.f * (1.f - kb),
      visual_map.scale_x, visual_map.scale_y, visual_map.offset_x, visual_map.offset_y,
      static_cast<float>(transform.opacity), 0.f, 0.f, 0.f,
  };
  context_->UpdateSubresource(video_params_, 0, nullptr, params, 0, 0);
  ID3D11ShaderResourceView* srvs[2] = {still ? srv_still_ : srv_y_, still ? nullptr : srv_uv_};
  context_->PSSetShaderResources(0, 2, srvs);
  if (still) context_->PSSetShader(ps_still_, nullptr, 0);
  context_->Draw(3, 0);
  if (still) context_->PSSetShader(ps_2d_, nullptr, 0);
  ID3D11ShaderResourceView* null_srvs[2] = {nullptr, nullptr};
  context_->PSSetShaderResources(0, 2, null_srvs);
  return true;
}

bool GpuPresenter::presentLayers(const DecoderFrame* frames, const VisualTransform* transforms,
                                 size_t count) {
  if ((count > 0 && (!frames || !transforms)) || !device_ || !context_ || !hwnd_) return false;
  if (!EnsurePipeline()) return false;
  RECT rc{};
  GetClientRect(hwnd_, &rc);
  const int cw = (std::max)(1L, rc.right - rc.left);
  const int ch = (std::max)(1L, rc.bottom - rc.top);
  if (!EnsureSwapChain(cw, ch) || !rtv_) return false;

  D3D11_VIEWPORT vp{};
  vp.Width = static_cast<float>(width_);
  vp.Height = static_cast<float>(height_);
  vp.MaxDepth = 1.f;
  context_->OMSetRenderTargets(1, &rtv_, nullptr);
  const float black[4] = {0.f, 0.f, 0.f, 1.f};
  context_->ClearRenderTargetView(rtv_, black);
  context_->RSSetViewports(1, &vp);
  context_->RSSetState(rast_);
  context_->OMSetBlendState(blend_, nullptr, 0xffffffff);
  context_->IASetInputLayout(nullptr);
  context_->IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
  context_->VSSetShader(vs_, nullptr, 0);
  context_->PSSetShader(ps_2d_, nullptr, 0);
  context_->PSSetConstantBuffers(0, 1, &video_params_);
  context_->PSSetSamplers(0, 1, &sampler_);
  DrawViewport(false);

  bool drew = false;
  for (size_t i = 0; i < count; ++i) {
    if (DrawLayer(frames[i], transforms[i])) drew = true;
  }
  DrawViewport(true);
  if (count > 0 && (!viewport_.enabled || viewport_.show_outside)) {
    DrawGizmo();
    DrawGuides();
  }
  if (!drew && count == 0) {
    return SUCCEEDED(swap_->Present(0, 0));
  }
  return drew && SUCCEEDED(swap_->Present(0, 0));
}

bool GpuPresenter::present(const DecoderFrame& frame) {
  return presentLayers(&frame, &transform_, 1);
}
