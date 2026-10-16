#pragma once

inline constexpr char kGizmoPixelShader[] = R"(
struct VSOut { float4 pos : SV_Position; float2 uv : TEXCOORD0; };
cbuffer GizmoParams : register(b0) {
  float4 bounds;
  float4 viewportStyle;
  float4 flags;
};
bool nearHandle(float2 p, float2 center) {
  return all(abs(p - center) <= viewportStyle.w);
}
float4 PSMain(VSOut i) : SV_Target {
  float2 lo = bounds.xy * viewportStyle.xy;
  float2 hi = bounds.zw * viewportStyle.xy;
  float2 middle = lo * 0.5 + hi * 0.5;
  float2 p = i.pos.xy;
  float edge = viewportStyle.z;
  bool horizontal = (abs(p.y - lo.y) <= edge || abs(p.y - hi.y) <= edge) &&
                    p.x >= lo.x - edge && p.x <= hi.x + edge;
  bool vertical = (abs(p.x - lo.x) <= edge || abs(p.x - hi.x) <= edge) &&
                  p.y >= lo.y - edge && p.y <= hi.y + edge;
  bool handle = flags.x > 0.5 && (
      nearHandle(p, lo) || nearHandle(p, float2(hi.x, lo.y)) ||
      nearHandle(p, hi) || nearHandle(p, float2(lo.x, hi.y)) ||
      nearHandle(p, float2(lo.x, middle.y)) || nearHandle(p, float2(hi.x, middle.y)) ||
      nearHandle(p, float2(middle.x, lo.y)) || nearHandle(p, float2(middle.x, hi.y)));
  if (handle) return float4(0.90, 0.96, 1.0, 1.0);
  if (horizontal || vertical) return float4(0.20, 0.62, 1.0, 1.0);
  discard;
  return float4(0.0, 0.0, 0.0, 0.0);
}
)";
