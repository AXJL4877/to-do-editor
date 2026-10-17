#define NOMINMAX
#include <windows.h>
#include <d3d11.h>
#include <d3dcompiler.h>
#include <wrl/client.h>
#include <array>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <stdexcept>

#include "transform/visual_transform.h"
#include "transform/visual_transform_shader.h"
#include "transform/transform_gizmo_shader.h"
#include "viewport/preview_viewport.h"
#include "viewport/preview_viewport_shader.h"

using Microsoft::WRL::ComPtr;

namespace {
int failures = 0;
void Check(bool condition, const char* name) {
  if (!condition) { std::fprintf(stderr, "FAIL: %s\n", name); ++failures; }
}
void Require(HRESULT result) {
  if (FAILED(result)) throw std::runtime_error("GPU regression setup failed");
}
ComPtr<ID3DBlob> Compile(const char* source, const char* entry, const char* target) {
  ComPtr<ID3DBlob> result, errors;
  const HRESULT hr = D3DCompile(source, std::strlen(source), nullptr, nullptr, nullptr,
      entry, target, 0, 0, &result, &errors);
  if (errors) std::fprintf(stderr, "%.*s", static_cast<int>(errors->GetBufferSize()),
      static_cast<const char*>(errors->GetBufferPointer()));
  Require(hr);
  return result;
}

// Runs the exact shaders used by GpuPresenter on WARP and inspects real pixels.
// NV12 plane views are represented by equivalent R8/R8G8 textures here.
struct GpuTest {
  ComPtr<ID3D11Device> device;
  ComPtr<ID3D11DeviceContext> context;
  ComPtr<ID3D11Texture2D> target, readback;
  ComPtr<ID3D11RenderTargetView> rtv;
  ComPtr<ID3D11VertexShader> vertex;
  ComPtr<ID3D11PixelShader> video, still, gizmo, packed, reveal;
  ComPtr<ID3D11PixelShader> stage;
  ComPtr<ID3D11Buffer> stage_params;
  ComPtr<ID3D11Buffer> video_params, gizmo_params;
  ComPtr<ID3D11ShaderResourceView> y_view, uv_view, still_view;
  ComPtr<ID3D11SamplerState> sampler;
  ComPtr<ID3D11RasterizerState> rasterizer;
  ComPtr<ID3D11BlendState> blend;

  GpuTest() {
    Require(D3D11CreateDevice(nullptr, D3D_DRIVER_TYPE_WARP, nullptr, 0, nullptr, 0,
        D3D11_SDK_VERSION, &device, nullptr, &context));
    auto vs = Compile(kVisualVertexShader, "VSMain", "vs_4_0");
    auto ps = Compile(kVideoPixelShader, "PSMain", "ps_4_0");
    auto sp = Compile(kStillPixelShader, "PSMain", "ps_4_0");
    auto pp = Compile(kPackedPixelShader, "PSMain", "ps_4_0");
    auto rp = Compile(kRevealPixelShader, "PSMain", "ps_4_0");
    Require(device->CreatePixelShader(pp->GetBufferPointer(), pp->GetBufferSize(), nullptr, &packed));
    Require(device->CreatePixelShader(rp->GetBufferPointer(), rp->GetBufferSize(), nullptr, &reveal));
    auto gs = Compile(kGizmoPixelShader, "PSMain", "ps_4_0");
    auto ss = Compile(kViewportPixelShader, "PSMain", "ps_4_0");
    Require(device->CreateVertexShader(vs->GetBufferPointer(), vs->GetBufferSize(), nullptr, &vertex));
    Require(device->CreatePixelShader(ps->GetBufferPointer(), ps->GetBufferSize(), nullptr, &video));
    Require(device->CreatePixelShader(sp->GetBufferPointer(), sp->GetBufferSize(), nullptr, &still));
    Require(device->CreatePixelShader(gs->GetBufferPointer(), gs->GetBufferSize(), nullptr, &gizmo));
    Require(device->CreatePixelShader(ss->GetBufferPointer(), ss->GetBufferSize(), nullptr, &stage));
    D3D11_TEXTURE2D_DESC desc{};
    desc.Width = desc.Height = 64;
    desc.MipLevels = desc.ArraySize = 1;
    desc.Format = DXGI_FORMAT_R8G8B8A8_UNORM;
    desc.SampleDesc.Count = 1;
    desc.Usage = D3D11_USAGE_DEFAULT;
    desc.BindFlags = D3D11_BIND_RENDER_TARGET;
    Require(device->CreateTexture2D(&desc, nullptr, &target));
    Require(device->CreateRenderTargetView(target.Get(), nullptr, &rtv));
    desc.Usage = D3D11_USAGE_STAGING;
    desc.BindFlags = 0;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    Require(device->CreateTexture2D(&desc, nullptr, &readback));
    D3D11_BUFFER_DESC cb{};
    cb.ByteWidth = 24 * sizeof(float);
    cb.Usage = D3D11_USAGE_DEFAULT;
    cb.BindFlags = D3D11_BIND_CONSTANT_BUFFER;
    Require(device->CreateBuffer(&cb, nullptr, &video_params));
    cb.ByteWidth = 12 * sizeof(float);
    Require(device->CreateBuffer(&cb, nullptr, &gizmo_params));
    cb.ByteWidth = 8 * sizeof(float);
    Require(device->CreateBuffer(&cb, nullptr, &stage_params));
    desc.Width = desc.Height = 1;
    desc.Usage = D3D11_USAGE_IMMUTABLE;
    desc.CPUAccessFlags = 0;
    desc.BindFlags = D3D11_BIND_SHADER_RESOURCE;
    const unsigned char white = 255;
    desc.Format = DXGI_FORMAT_R8_UNORM;
    D3D11_SUBRESOURCE_DATA data{&white, 1, 0};
    ComPtr<ID3D11Texture2D> y_texture, uv_texture;
    Require(device->CreateTexture2D(&desc, &data, &y_texture));
    Require(device->CreateShaderResourceView(y_texture.Get(), nullptr, &y_view));
    const unsigned char neutral[] = {128, 128};
    desc.Format = DXGI_FORMAT_R8G8_UNORM;
    data = {neutral, 2, 0};
    Require(device->CreateTexture2D(&desc, &data, &uv_texture));
    Require(device->CreateShaderResourceView(uv_texture.Get(), nullptr, &uv_view));
    // 2x1 premultiplied BGRA still: left half-transparent red, right fully transparent.
    const unsigned char translucent[] = {0, 0, 128, 128, 0, 0, 0, 0};
    desc.Width = 2;
    desc.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
    data = {translucent, 8, 0};
    ComPtr<ID3D11Texture2D> still_texture;
    Require(device->CreateTexture2D(&desc, &data, &still_texture));
    Require(device->CreateShaderResourceView(still_texture.Get(), nullptr, &still_view));
    D3D11_SAMPLER_DESC sample{};
    sample.Filter = D3D11_FILTER_MIN_MAG_MIP_LINEAR;
    sample.AddressU = sample.AddressV = sample.AddressW = D3D11_TEXTURE_ADDRESS_CLAMP;
    sample.MaxLOD = D3D11_FLOAT32_MAX;
    Require(device->CreateSamplerState(&sample, &sampler));
    D3D11_RASTERIZER_DESC raster{};
    raster.FillMode = D3D11_FILL_SOLID;
    raster.CullMode = D3D11_CULL_NONE;
    raster.DepthClipEnable = TRUE;
    Require(device->CreateRasterizerState(&raster, &rasterizer));
    D3D11_BLEND_DESC blend_desc{};
    blend_desc.RenderTarget[0].BlendEnable = TRUE;
    blend_desc.RenderTarget[0].SrcBlend = D3D11_BLEND_ONE;
    blend_desc.RenderTarget[0].DestBlend = D3D11_BLEND_INV_SRC_ALPHA;
    blend_desc.RenderTarget[0].BlendOp = D3D11_BLEND_OP_ADD;
    blend_desc.RenderTarget[0].SrcBlendAlpha = D3D11_BLEND_ONE;
    blend_desc.RenderTarget[0].DestBlendAlpha = D3D11_BLEND_INV_SRC_ALPHA;
    blend_desc.RenderTarget[0].BlendOpAlpha = D3D11_BLEND_OP_ADD;
    blend_desc.RenderTarget[0].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL;
    Require(device->CreateBlendState(&blend_desc, &blend));
    ID3D11RenderTargetView* output = rtv.Get();
    context->OMSetRenderTargets(1, &output, nullptr);
    D3D11_VIEWPORT viewport{0, 0, 64, 64, 0, 1};
    context->RSSetViewports(1, &viewport);
    context->RSSetState(rasterizer.Get());
    context->OMSetBlendState(blend.Get(), nullptr, 0xffffffff);
    context->IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    context->VSSetShader(vertex.Get(), nullptr, 0);
  }

  void Clear() {
    const float black[4] = {0.f, 0.f, 0.f, 1.f};
    context->ClearRenderTargetView(rtv.Get(), black);
  }

  void Image(VisualTransform transform = {}, PreviewViewport viewport = {}) {
    const auto map = MapViewportVisual(FitVisualTransform(
        64 * viewport.width, 64 * viewport.height, 64, 64, transform), viewport);
    const float params[24] = {1, 1, 0.5f, 0.5f, 0, 0, 0, 0,
        0, 1, 128.f / 255, 1, 0, 0, 0, 0,
        map.scale_x, map.scale_y, map.offset_x, map.offset_y,
        static_cast<float>(transform.opacity), 0, 0, 0};
    context->UpdateSubresource(video_params.Get(), 0, nullptr, params, 0, 0);
    ID3D11Buffer* buffer = video_params.Get();
    context->PSSetConstantBuffers(0, 1, &buffer);
    context->PSSetShader(video.Get(), nullptr, 0);
    ID3D11ShaderResourceView* views[] = {y_view.Get(), uv_view.Get()};
    context->PSSetShaderResources(0, 2, views);
    ID3D11SamplerState* sampling = sampler.Get();
    context->PSSetSamplers(0, 1, &sampling);
    context->Draw(3, 0);
  }

  // Point-sampled premultiplied still with the production shader and constants.
  void Still(float opacity) {
    const float params[24] = {1, 1, 0.25f, 0.5f, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0,
        1, 1, 0, 0,
        opacity, 0, 0, 0};
    context->UpdateSubresource(video_params.Get(), 0, nullptr, params, 0, 0);
    ID3D11Buffer* buffer = video_params.Get();
    context->PSSetConstantBuffers(0, 1, &buffer);
    context->PSSetShader(still.Get(), nullptr, 0);
    ID3D11ShaderResourceView* views[] = {still_view.Get(), nullptr};
    context->PSSetShaderResources(0, 2, views);
    ID3D11SamplerState* sampling = sampler.Get();
    context->PSSetSamplers(0, 1, &sampling);
    context->Draw(3, 0);
  }

  ComPtr<ID3D11ShaderResourceView> Texture(DXGI_FORMAT format, UINT width, UINT height,
                                           const void* pixels, UINT pitch) {
    D3D11_TEXTURE2D_DESC desc{};
    desc.Width = width;
    desc.Height = height;
    desc.MipLevels = desc.ArraySize = 1;
    desc.Format = format;
    desc.SampleDesc.Count = 1;
    desc.Usage = D3D11_USAGE_IMMUTABLE;
    desc.BindFlags = D3D11_BIND_SHADER_RESOURCE;
    D3D11_SUBRESOURCE_DATA data{pixels, pitch, 0};
    ComPtr<ID3D11Texture2D> texture;
    Require(device->CreateTexture2D(&desc, &data, &texture));
    ComPtr<ID3D11ShaderResourceView> view;
    Require(device->CreateShaderResourceView(texture.Get(), nullptr, &view));
    return view;
  }

  // 4x1 reveal atlas: two white picture texels, then their times 0.25 / 0.75.
  void Reveal(float progress, float soft, unsigned char edge) {
    const unsigned char atlas[] = {255, 255, 255, 255, 255, 255, 255, 255,
                                   0, 64, 0, 255,       // B=soft 0, G=lo, R=hi: t=0.25
                                   edge, 255, 191, 255};  // t~0.75, soft from `edge`
    auto view = Texture(DXGI_FORMAT_B8G8R8A8_UNORM, 4, 1, atlas, 16);
    const float params[24] = {0.5f, 1, 0.5f / 4, 0.5f, 0, 0, 0.5f, 0,
        0, 0, 0, 0, 0, 0, 0, 0,
        1, 1, 0, 0,
        1, progress, soft, 0};
    context->UpdateSubresource(video_params.Get(), 0, nullptr, params, 0, 0);
    ID3D11Buffer* buffer = video_params.Get();
    context->PSSetConstantBuffers(0, 1, &buffer);
    context->PSSetShader(reveal.Get(), nullptr, 0);
    ID3D11ShaderResourceView* views[] = {view.Get(), nullptr};
    context->PSSetShaderResources(0, 2, views);
    ID3D11SamplerState* sampling = sampler.Get();
    context->PSSetSamplers(0, 1, &sampling);
    context->Draw(3, 0);
  }

  // Packed alpha: luma white on top, 50% alpha luma below; neutral chroma.
  void Packed() {
    const unsigned char luma[] = {255, 128};
    auto y = Texture(DXGI_FORMAT_R8_UNORM, 1, 2, luma, 1);
    const unsigned char chroma[] = {128, 128};
    auto uv = Texture(DXGI_FORMAT_R8G8_UNORM, 1, 1, chroma, 2);
    const float params[24] = {1, 0.5f, 0.5f, 0.25f, 0, 0, 0, 0.5f,
        0, 1, 128.f / 255, 1, 0, 0, 0, 0,
        1, 1, 0, 0,
        1, 0, 0, 0};
    context->UpdateSubresource(video_params.Get(), 0, nullptr, params, 0, 0);
    ID3D11Buffer* buffer = video_params.Get();
    context->PSSetConstantBuffers(0, 1, &buffer);
    context->PSSetShader(packed.Get(), nullptr, 0);
    ID3D11ShaderResourceView* views[] = {y.Get(), uv.Get()};
    context->PSSetShaderResources(0, 2, views);
    ID3D11SamplerState* sampling = sampler.Get();
    context->PSSetSamplers(0, 1, &sampling);
    context->Draw(3, 0);
  }

  void Stage(const PreviewViewport& viewport, bool overlay) {
    const float params[8] = {static_cast<float>(viewport.left), static_cast<float>(viewport.top),
        static_cast<float>(viewport.width), static_cast<float>(viewport.height),
        64, 64, overlay ? 1.f : 0.f, viewport.show_outside ? 1.f : 0.f};
    context->UpdateSubresource(stage_params.Get(), 0, nullptr, params, 0, 0);
    ID3D11Buffer* buffer = stage_params.Get();
    context->PSSetConstantBuffers(0, 1, &buffer);
    context->PSSetShader(stage.Get(), nullptr, 0);
    context->Draw(3, 0);
  }

  void Selection(bool editable, float dpi_scale = 1,
                 std::array<float, 4> bounds = {0.25f, 0.25f, 0.75f, 0.75f}) {
    const float params[12] = {bounds[0], bounds[1], bounds[2], bounds[3],
        64, 64, 0.75f * dpi_scale, 4 * dpi_scale, editable ? 1.f : 0.f, 0, 0, 0};
    context->UpdateSubresource(gizmo_params.Get(), 0, nullptr, params, 0, 0);
    ID3D11Buffer* buffer = gizmo_params.Get();
    context->PSSetConstantBuffers(0, 1, &buffer);
    context->PSSetShader(gizmo.Get(), nullptr, 0);
    context->Draw(3, 0);
  }

  std::array<unsigned char, 4> Pixel(int x, int y) {
    context->CopyResource(readback.Get(), target.Get());
    D3D11_MAPPED_SUBRESOURCE mapped{};
    Require(context->Map(readback.Get(), 0, D3D11_MAP_READ, 0, &mapped));
    const auto* pixel = static_cast<const unsigned char*>(mapped.pData) + y * mapped.RowPitch + x * 4;
    std::array<unsigned char, 4> result{pixel[0], pixel[1], pixel[2], pixel[3]};
    context->Unmap(readback.Get(), 0);
    return result;
  }
};
}

int main() {
  try {
    GpuTest gpu;
    // Reveal: the left pixel (t=0.25) shows at progress 0.5, the right (t=0.75) not yet.
    gpu.Clear();
    gpu.Reveal(0.5f, 0.f, 0);
    Check(gpu.Pixel(4, 0)[0] == 255, "revealed pixel is drawn");
    Check(gpu.Pixel(60, 0)[0] == 0, "pixel not yet revealed stays transparent");
    gpu.Clear();
    gpu.Reveal(1.f, 0.f, 0);
    Check(gpu.Pixel(4, 0)[0] == 255 && gpu.Pixel(60, 0)[0] == 255, "everything shows at the end");
    gpu.Clear();
    gpu.Reveal(0.f, 0.f, 0);
    Check(gpu.Pixel(4, 0)[0] == 0 && gpu.Pixel(60, 0)[0] == 0, "nothing shows at the start");
    // Soft edge: B=255 with soft 0.5 fades in over [0.25, 0.75]; halfway is about half.
    gpu.Clear();
    gpu.Reveal(0.5f, 0.5f, 255);
    const int faded = gpu.Pixel(60, 0)[0];
    Check(faded > 90 && faded < 165, "soft edge fades the pixel in");
    // Packed: white at 50% packed alpha over black is mid grey.
    gpu.Clear();
    gpu.Packed();
    const int grey = gpu.Pixel(32, 32)[0];
    Check(std::abs(grey - 128) <= 3, "packed alpha premultiplies the colour");
    VisualTransform transform;
    gpu.Clear();
    gpu.Image(transform);
    Check(gpu.Pixel(32, 32)[0] == 255, "opaque video is white");
    gpu.Clear();
    transform.opacity = 0.5;
    gpu.Image(transform);
    const auto half = gpu.Pixel(32, 32);
    Check(std::abs(static_cast<int>(half[0]) - 128) <= 1 && half[0] == half[1] && half[1] == half[2],
          "half opacity visibly halves RGB over black");
    // Premultiplied blend onto opaque black keeps destination alpha at 1.
    Check(half[3] == 255, "composited swap-chain pixel stays opaque");
    gpu.Clear();
    transform.opacity = 0;
    gpu.Image(transform);
    Check(gpu.Pixel(32, 32)[0] == 0, "zero opacity hides image");
    gpu.Clear();
    gpu.Image(transform);
    gpu.Selection(true);
    Check(gpu.Pixel(24, 16)[2] == 255, "selection outline remains at zero image opacity");
    for (auto center : {std::array<int, 2>{16, 16}, {48, 16}, {48, 48}, {16, 48},
                        {16, 32}, {48, 32}, {32, 16}, {32, 48}}) {
      Check(gpu.Pixel(center[0] + 3, center[1] + 3)[0] > 220, "all eight editable handles render");
    }
    gpu.Clear();
    gpu.Image(transform);
    gpu.Selection(false);
    Check(gpu.Pixel(19, 19)[0] == 0 && gpu.Pixel(24, 16)[2] == 255,
          "locked selection keeps outline and omits handles");
    gpu.Clear();
    gpu.Image(transform);
    gpu.Selection(true, 2);
    Check(gpu.Pixel(22, 22)[0] > 220, "handle square grows with window DPI");
    gpu.Clear();
    gpu.Image(transform);
    gpu.Selection(true, 1, {-0.25f, 0.25f, 0.5f, 0.75f});
    Check(gpu.Pixel(32, 24)[2] == 255 && gpu.Pixel(35, 35)[0] > 220,
          "off-canvas bounds keep their visible edge and midpoint handle");
    transform = {};
    transform.scale_x = 0.5;
    gpu.Clear();
    gpu.Image(transform);
    Check(gpu.Pixel(8, 32)[0] == 0 && gpu.Pixel(32, 8)[0] == 255,
          "horizontal axis scale changes only width");
    transform = {};
    transform.scale_y = 0.5;
    gpu.Clear();
    gpu.Image(transform);
    Check(gpu.Pixel(32, 8)[0] == 0 && gpu.Pixel(8, 32)[0] == 255,
          "vertical axis scale changes only height");

    // Translucent still over an opaque white layer: alpha shows the layer below.
    transform = {};
    gpu.Clear();
    gpu.Image(transform);
    gpu.Still(1.f);
    const auto mixed = gpu.Pixel(8, 32);
    Check(mixed[0] == 255 && std::abs(static_cast<int>(mixed[1]) - 127) <= 2,
          "half-transparent red still blends over the lower layer");
    Check(gpu.Pixel(56, 32)[1] == 255, "transparent still pixels leave the lower layer intact");
    gpu.Clear();
    gpu.Image(transform);
    gpu.Still(0.f);
    Check(gpu.Pixel(8, 32)[1] == 255, "zero layer opacity hides a still entirely");

    PreviewViewport viewport;
    Check(ParsePreviewViewport("0.25 0.25 0.5 0.5 1", viewport), "editing viewport setup");
    transform = {};
    transform.x = -0.75;
    transform.scale = 0.25;
    gpu.Stage(viewport, false);
    gpu.Image(transform, viewport);
    gpu.Stage(viewport, true);
    Check(gpu.Pixel(8, 32)[0] > 100 && gpu.Pixel(8, 32)[0] < 240,
          "outside element remains visible with editing tint");
    Check(gpu.Pixel(32, 32)[0] == 0, "empty output frame remains black");
    Check(gpu.Pixel(16, 24)[0] > 30, "output frame has a visible editing border");
    viewport.show_outside = false;
    gpu.Stage(viewport, false);
    gpu.Image(transform, viewport);
    gpu.Stage(viewport, true);
    Check(gpu.Pixel(8, 32)[0] < 40, "output-only view masks off-frame elements");
    transform = {};
    gpu.Stage(viewport, false);
    gpu.Image(transform, viewport);
    gpu.Stage(viewport, true);
    Check(gpu.Pixel(32, 32)[0] == 255 && gpu.Pixel(8, 32)[0] < 40,
          "output-only view preserves inside pixels and excludes outside pixels");
  } catch (const std::exception& error) {
    std::fprintf(stderr, "%s\n", error.what());
    return 1;
  }
  return failures ? 1 : 0;
}
