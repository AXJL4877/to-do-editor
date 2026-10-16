#pragma once

#include "decoder.h"
#include "transform/align_guides.h"
#include "transform/visual_transform.h"
#include "transform/transform_gizmo.h"
#include "viewport/preview_viewport.h"

// Draw decoder GPU texture to HWND via DXGI SwapChain + fullscreen triangle
class GpuPresenter {
 public:
  GpuPresenter();
  ~GpuPresenter();

  bool bind(HWND hwnd, ID3D11Device* device, ID3D11DeviceContext* context);
  // CmdSize / WM_SIZE: ResizeBuffers when dimensions change
  void resize(int width, int height);
  bool present(const DecoderFrame& frame);
  /// Draw layers bottom→top. Each entry supplies its own transform; clears once.
  bool presentLayers(const DecoderFrame* frames, const VisualTransform* transforms, size_t count);
  void clear();
  bool setTransform(const VisualTransform& transform);
  bool setGizmo(const std::optional<TransformGizmo>& gizmo);
  bool setGuides(std::vector<AlignGuide> guides);
  bool setViewport(const PreviewViewport& viewport);
  const std::optional<TransformGizmo>& gizmo() const { return gizmo_; }
  const std::vector<AlignGuide>& guides() const { return guides_; }

 private:
  bool EnsureSwapChain(int width, int height);
  bool EnsurePipeline();
  bool EnsureRtv();
  bool EnsureSamplingTexture(const D3D11_TEXTURE2D_DESC& source);
  bool EnsureStillTexture(const D3D11_TEXTURE2D_DESC& source);
  bool DrawLayer(const DecoderFrame& frame, const VisualTransform& transform);
  void ReleaseSizeDependent();
  void ReleaseSamplingTexture();
  void ReleaseStillTexture();
  void ReleasePipeline();
  void DrawGizmo();
  void DrawGuides();
  void DrawViewport(bool overlay);

  HWND hwnd_ = nullptr;
  ID3D11Device* device_ = nullptr;
  ID3D11DeviceContext* context_ = nullptr;
  IDXGISwapChain1* swap_ = nullptr;
  ID3D11RenderTargetView* rtv_ = nullptr;
  ID3D11VertexShader* vs_ = nullptr;
  ID3D11PixelShader* ps_2d_ = nullptr;
  ID3D11PixelShader* ps_still_ = nullptr;
  ID3D11Buffer* video_params_ = nullptr;
  ID3D11PixelShader* ps_gizmo_ = nullptr;
  ID3D11Buffer* gizmo_params_ = nullptr;
  ID3D11PixelShader* ps_guides_ = nullptr;
  ID3D11Buffer* guides_params_ = nullptr;
  ID3D11PixelShader* ps_viewport_ = nullptr;
  ID3D11Buffer* viewport_params_ = nullptr;
  ID3D11Texture2D* sampling_texture_ = nullptr;
  ID3D11ShaderResourceView* srv_y_ = nullptr;
  ID3D11ShaderResourceView* srv_uv_ = nullptr;
  ID3D11Texture2D* still_texture_ = nullptr;
  ID3D11ShaderResourceView* srv_still_ = nullptr;
  ID3D11SamplerState* sampler_ = nullptr;
  ID3D11RasterizerState* rast_ = nullptr;
  ID3D11BlendState* blend_ = nullptr;
  int width_ = 0;
  int height_ = 0;
  VisualTransform transform_;
  std::optional<TransformGizmo> gizmo_;
  std::vector<AlignGuide> guides_;
  PreviewViewport viewport_;
};
