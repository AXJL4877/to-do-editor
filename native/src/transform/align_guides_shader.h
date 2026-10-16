#pragma once

inline constexpr char kAlignGuidesPixelShader[] = R"(
struct VSOut { float4 pos : SV_Position; float2 uv : TEXCOORD0; };
cbuffer GuideParams : register(b0) {
  float4 positions;
  float4 axes;
  float4 style;
};
float4 PSMain(VSOut i) : SV_Target {
  float2 p = i.pos.xy;
  float thickness = style.z;
  float count = style.w;
  [unroll]
  for (int n = 0; n < 4; ++n) {
    if (n >= (int)count) break;
    float axis = axes[n];
    float pos = positions[n];
    bool hit = axis < 0.5
        ? abs(p.x - pos * style.x) <= thickness
        : abs(p.y - pos * style.y) <= thickness;
    if (hit) return float4(1.0, 0.35, 0.75, 1.0);
  }
  discard;
  return float4(0.0, 0.0, 0.0, 0.0);
}
)";
