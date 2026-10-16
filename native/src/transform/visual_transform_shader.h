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
