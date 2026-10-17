#pragma once

// Shared with the GPU regression harness so tests execute the production shader.
inline constexpr char kVisualVertexShader[] = R"(
struct VSOut { float4 pos : SV_Position; float2 uv : TEXCOORD0; };
VSOut VSMain(uint id : SV_VertexID) {
  VSOut o;
  float2 uv = float2((id << 1) & 2, id & 2);
  o.pos = float4(uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
  o.uv = uv;
  return o;
}
)";

inline constexpr char kVideoPixelShader[] = R"(
struct VSOut { float4 pos : SV_Position; float2 uv : TEXCOORD0; };
Texture2D texY : register(t0);
Texture2D texUV : register(t1);
SamplerState samp : register(s0);
cbuffer VideoParams : register(b0) {
  float4 uvScale;
  float4 uvOffset;
  float4 range;
  float4 coefficients;
  float4 visualMap;
  float4 visualStyle;
};
float4 PSMain(VSOut i) : SV_Target {
  float2 videoUv = i.uv * visualMap.xy + visualMap.zw;
  // Transparent outside the fitted picture so upper layers do not paint black
  // over lower tracks during multi-layer compositing.
  if (any(videoUv < 0.0) || any(videoUv > 1.0)) return float4(0.0, 0.0, 0.0, 0.0);
  float2 coord = clamp(uvOffset.xy + videoUv * uvScale.xy,
                      uvOffset.xy + uvScale.zw, uvOffset.xy + uvScale.xy - uvScale.zw);
  float2 chromaInset = min(uvScale.zw * 2.0, uvScale.xy * 0.5);
  float2 chromaCoord = clamp(uvOffset.xy + videoUv * uvScale.xy,
                            uvOffset.xy + chromaInset, uvOffset.xy + uvScale.xy - chromaInset);
  float y = (texY.Sample(samp, coord).r - range.x) * range.y;
  float2 uv = (texUV.Sample(samp, chromaCoord).rg - range.z) * range.w;
  float r = y + coefficients.x * uv.y;
  float g = y - coefficients.y * uv.x - coefficients.z * uv.y;
  float b = y + coefficients.w * uv.x;
  float opacity = saturate(visualStyle.x);
  // Premultiplied RGB so layers can alpha-blend over a cleared black canvas.
  return float4(saturate(float3(r, g, b)) * opacity, opacity);
}
)";

// Packed-alpha video: colour in the top half, alpha as luma in the bottom
// half (`uvOffset.w` is the texture-space distance between them).
inline constexpr char kPackedPixelShader[] = R"(
struct VSOut { float4 pos : SV_Position; float2 uv : TEXCOORD0; };
Texture2D texY : register(t0);
Texture2D texUV : register(t1);
SamplerState samp : register(s0);
cbuffer VideoParams : register(b0) {
  float4 uvScale;
  float4 uvOffset;
  float4 range;
  float4 coefficients;
  float4 visualMap;
  float4 visualStyle;
};
float4 PSMain(VSOut i) : SV_Target {
  float2 videoUv = i.uv * visualMap.xy + visualMap.zw;
  if (any(videoUv < 0.0) || any(videoUv > 1.0)) return float4(0.0, 0.0, 0.0, 0.0);
  float2 coord = clamp(uvOffset.xy + videoUv * uvScale.xy,
                      uvOffset.xy + uvScale.zw, uvOffset.xy + uvScale.xy - uvScale.zw);
  float2 chromaInset = min(uvScale.zw * 2.0, uvScale.xy * 0.5);
  float2 chromaCoord = clamp(uvOffset.xy + videoUv * uvScale.xy,
                            uvOffset.xy + chromaInset, uvOffset.xy + uvScale.xy - chromaInset);
  float y = (texY.Sample(samp, coord).r - range.x) * range.y;
  float2 uv = (texUV.Sample(samp, chromaCoord).rg - range.z) * range.w;
  float r = y + coefficients.x * uv.y;
  float g = y - coefficients.y * uv.x - coefficients.z * uv.y;
  float b = y + coefficients.w * uv.x;
  float alpha = saturate((texY.Sample(samp, coord + float2(0.0, uvOffset.w)).r - range.x) * range.y);
  float shown = alpha * saturate(visualStyle.x);
  // Colour is straight; premultiply by the packed alpha.
  return float4(saturate(float3(r, g, b)) * shown, shown);
}
)";

// Reveal atlas (premultiplied BGRA): picture on the left, appearance times on
// the right at `uvOffset.z`. Timing texels hold 16-bit times and must not be
// filtered, so visibility is computed per texel and then blended.
inline constexpr char kRevealPixelShader[] = R"(
struct VSOut { float4 pos : SV_Position; float2 uv : TEXCOORD0; };
Texture2D texImage : register(t0);
SamplerState samp : register(s0);
cbuffer VideoParams : register(b0) {
  float4 uvScale;
  float4 uvOffset;
  float4 range;
  float4 coefficients;
  float4 visualMap;
  float4 visualStyle;
};
float Visible(int2 texel, float progress, float soft) {
  float4 t = texImage.Load(int3(texel, 0));
  if (t.a < 0.5) return 0.0;
  float reveal = (round(t.r * 255.0) * 256.0 + round(t.g * 255.0)) / 65535.0;
  if (progress >= reveal) return 1.0;
  float edge = t.b * soft;
  if (edge <= 1e-6 || progress <= reveal - edge) return 0.0;
  float u = saturate((progress - (reveal - edge)) / edge);
  return u * u * (3.0 - 2.0 * u);
}
float4 PSMain(VSOut i) : SV_Target {
  float2 videoUv = i.uv * visualMap.xy + visualMap.zw;
  if (any(videoUv < 0.0) || any(videoUv > 1.0)) return float4(0.0, 0.0, 0.0, 0.0);
  float2 coord = clamp(uvOffset.xy + videoUv * uvScale.xy,
                      uvOffset.xy + uvScale.zw, uvOffset.xy + uvScale.xy - uvScale.zw);
  float4 color = texImage.Sample(samp, coord);
  float2 size = 0.5 / uvScale.zw;
  float2 p = (coord + float2(uvOffset.z, 0.0)) * size - 0.5;
  int2 base = int2(floor(p));
  float2 f = p - floor(p);
  int2 lo = int2(round((uvOffset.xy + float2(uvOffset.z, 0.0)) * size));
  int2 hi = lo + int2(round(uvScale.xy * size)) - 1;
  float progress = visualStyle.y;
  float soft = visualStyle.z;
  float v00 = Visible(clamp(base, lo, hi), progress, soft);
  float v10 = Visible(clamp(base + int2(1, 0), lo, hi), progress, soft);
  float v01 = Visible(clamp(base + int2(0, 1), lo, hi), progress, soft);
  float v11 = Visible(clamp(base + int2(1, 1), lo, hi), progress, soft);
  float v = lerp(lerp(v00, v10, f.x), lerp(v01, v11, f.x), f.y);
  return color * (v * saturate(visualStyle.x));
}
)";

// Translucent stills (text rasters, PNG with alpha) arrive as premultiplied BGRA.
// Same constant layout as kVideoPixelShader; only uvScale/uvOffset/visual fields apply.
inline constexpr char kStillPixelShader[] = R"(
struct VSOut { float4 pos : SV_Position; float2 uv : TEXCOORD0; };
Texture2D texImage : register(t0);
SamplerState samp : register(s0);
cbuffer VideoParams : register(b0) {
  float4 uvScale;
  float4 uvOffset;
  float4 range;
  float4 coefficients;
  float4 visualMap;
  float4 visualStyle;
};
float4 PSMain(VSOut i) : SV_Target {
  float2 videoUv = i.uv * visualMap.xy + visualMap.zw;
  if (any(videoUv < 0.0) || any(videoUv > 1.0)) return float4(0.0, 0.0, 0.0, 0.0);
  float2 coord = clamp(uvOffset.xy + videoUv * uvScale.xy,
                      uvOffset.xy + uvScale.zw, uvOffset.xy + uvScale.xy - uvScale.zw);
  // Already premultiplied: scaling all four channels keeps it premultiplied.
  return texImage.Sample(samp, coord) * saturate(visualStyle.x);
}
)";
