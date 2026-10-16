#pragma once

// Shared by the presenter and WARP pixel regressions. The final pass masks or
// dims everything outside the composition without changing the layer textures.
inline constexpr char kViewportPixelShader[] = R"(
struct VSOut { float4 pos : SV_Position; float2 uv : TEXCOORD0; };
cbuffer ViewportParams : register(b0) {
  float4 canvas;
  float4 style;
};
float4 PSMain(VSOut i) : SV_Target {
  float2 p = (i.uv - canvas.xy) * style.xy;
  float2 size = canvas.zw * style.xy;
  bool inside = all(p >= 0.0) && all(p <= size);
  float3 background = float3(0.07, 0.075, 0.085);
  if (style.z < 0.5) return float4(inside ? float3(0.0, 0.0, 0.0) : background, 1.0);
  if (!inside) {
    if (style.w < 0.5) return float4(background, 1.0);
    return float4(float3(0.02, 0.025, 0.03) * 0.4, 0.4);
  }
  if (style.w > 0.5 && (any(p <= 1.0) || any(p >= size - 1.0)))
    return float4(0.28, 0.32, 0.38, 1.0);
  discard;
  return float4(0.0, 0.0, 0.0, 0.0);
}
)";
