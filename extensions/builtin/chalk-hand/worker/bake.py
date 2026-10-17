"""黑板手绘烘焙器（todo-bake-board/1）。

一块黑板 → 每个元素一个可编辑图层：
- 手写元素：用黑板报的逐区骨架书写渲染器（RegionStreamRenderer）真实书写一遍，
  不编码视频，而是记下每个像素出现的时刻，写成 todo-reveal/1 图集（定格画面 +
  出现时间 + 粉笔手轨迹）。宿主原生着色器逐帧精确回放，拖动零解码。
  自检：抽 8 帧把图集合成结果与渲染器真实画面比对，不合格时退回逐帧
  packed-alpha 视频（宿主 encode 服务编码）。
- 盖章 / 渐显元素：抠好的静帧 + 宿主入场动效（stamp / fade）。

元素图是黑底粉笔画：亮度即不透明度（to_straight_rgba），所以透明叠在任何底上都对。
随机数全部固定种子，同一输入产出逐字节一致。
"""
from __future__ import annotations

import copy
import json
import math
import os
import shutil
import sys
import zlib
from dataclasses import replace
from pathlib import Path

import cv2
import numpy as np
from PIL import Image, PngImagePlugin

from todo_worker import RunFailed, Worker

HERE = Path(__file__).resolve().parent
BLACKBOARD = HERE / "blackboard"
sys.path.insert(0, str(BLACKBOARD / "scripts"))

import render_stream_whiteboard as rsw  # noqa: E402
import stream_render as sr  # noqa: E402
from blackbg_element_hand import (  # noqa: E402
    LAYOUT_PRESETS,
    expressive_zones,
    layout_slot_rects,
    over,
    place_in_rect,
    zones_to_rects,
)
from soft_cutout import content_bbox, soft_cutout_bgr  # noqa: E402

# 透明输出：渲染器画在纯黑上，亮度即 alpha。
rsw._load_chalk_board = lambda h, w: np.zeros((h, w, 3), np.uint8)

BAKE_SPEC = "todo-bake-board/1"
REVEAL_SPEC = "todo-reveal/1"
ENTERS = ("draw", "stamp", "fade")
ROLE_BASE_MS = {"hero": 3200, "support": 2400, "accent": 1500}
# 写完后多留几帧完成画面，接定格时不闪
DRAW_TAIL_MS = 200
CROP_PAD = 8
TIP_RATE = 15
SOFT_MAX = 0.6
# 出现时刻按这些绝对 alpha 档位记录（单调书写下足以重建每个像素的显现曲线）
LEVELS = np.array([0.03, 0.08, 0.15, 0.25, 0.4, 0.55, 0.7, 0.85, 0.95], np.float32)
HAND_FILE = "hand_chalk.png"

worker = Worker(version="4.1.0")


# ── 基础 ────────────────────────────────────────────────────────────


def load_bgr(path: Path) -> np.ndarray:
    img = cv2.imdecode(np.fromfile(str(path), dtype=np.uint8), cv2.IMREAD_COLOR)
    if img is None:
        raise RunFailed(f"读不到元素图 {path.name}", "invalid_input")
    return img


def save_png(path: Path, img: np.ndarray) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    ok, data = cv2.imencode(".png", img)
    if not ok:
        raise RuntimeError(f"写不出 {path.name}")
    data.tofile(str(path))


def to_straight_rgba(bgr: np.ndarray, tone: str = "chalk") -> tuple[np.ndarray, np.ndarray]:
    """黑底粉笔 → 直通 alpha：亮度即不透明度，颜色反预乘。ink：只留色相，成深色墨线。"""
    f = bgr.astype(np.float32)
    a = np.clip(f.max(axis=2) / 170.0, 0.0, 1.0)
    color = np.clip(f / np.maximum(a, 1e-3)[..., None], 0, 255)
    if tone == "ink":
        color = color - color.min(axis=2, keepdims=True) * 0.85
    color[a <= 1e-3] = 0
    return color, a


def bgra(color: np.ndarray, alpha: np.ndarray) -> np.ndarray:
    return np.dstack([np.clip(color, 0, 255), np.clip(alpha, 0, 1) * 255.0]).astype(np.uint8)


def smoothstep(x: np.ndarray) -> np.ndarray:
    x = np.clip(x, 0.0, 1.0)
    return x * x * (3.0 - 2.0 * x)


# ── 版式与节奏（沿用旧渲染端规则）──────────────────────────────────


def layout_rects(n: int, W: int, H: int, layout: str):
    if n == 1:
        return "single", [(int(W * 0.05), int(H * 0.05), int(W * 0.95), int(H * 0.95))]
    portrait = H > W * 1.15
    if layout == "grid" or (layout == "auto" and portrait):
        cols = 1 if portrait else (3 if n >= 5 else 2 if n == 4 else n)
        return "grid", layout_slot_rects(n, W, H, layout="grid", cols=cols)
    preset = layout if layout in LAYOUT_PRESETS else {2: "hero_left", 3: "hero_left", 4: "fork_right"}.get(n, "climax_right")
    return preset, zones_to_rects(expressive_zones(n, preset), W, H)


def cut(path: Path) -> np.ndarray:
    rgba = soft_cutout_bgr(load_bgr(path))
    x0, y0, x1, y1 = content_bbox(rgba)
    crop = rgba[y0:y1, x0:x1].copy()
    if crop.size == 0 or int(crop[:, :, 3].max()) < 20:
        raise RunFailed(f"元素图 {path.name} 抠图后为空（不是黑底粉笔画？）", "invalid_input")
    return crop


def place(els, crops, W, H, layout, mask_dir: Path):
    """叙事模板槽位排版；每元素一张黑底单独画面和书写用蒙版。"""
    preset, rects = layout_rects(len(els), W, H, layout)
    weights = [float(el.get("weight", 1.0)) for el in els]
    w_max = max(weights) or 1.0
    placed, singles = [], []
    for el, crop, rect, wt in zip(els, crops, rects, weights):
        fill = float(np.clip(0.88 * (0.92 + 0.10 * wt / w_max), 0.82, 0.98))
        pl = place_in_rect(crop, rect, H, W, fill=fill)
        layer = np.zeros((H, W, 4), np.uint8)
        layer[pl.y:pl.y + pl.height, pl.x:pl.x + pl.width] = cv2.resize(crop, (pl.width, pl.height), interpolation=cv2.INTER_AREA)
        singles.append(over(np.zeros((H, W, 3), np.uint8), layer))
        mask = (layer[:, :, 3] > 28).astype(np.uint8) * 255
        mask = cv2.dilate(mask, cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (5, 5)), iterations=1)
        ys, xs = np.where(mask > 0)
        bx0, by0, bx1, by1 = int(xs.min()), int(ys.min()), int(xs.max()) + 1, int(ys.max()) + 1
        save_png(mask_dir / f"{el['id']}.png", mask[by0:by1, bx0:bx1])
        placed.append({"id": el["id"], "label": el.get("label") or "", "role": el.get("role") or "support",
                       "box": [bx0, by0, bx1 - bx0, by1 - by0], "area": int((mask > 0).sum())})
    return preset, placed, singles


def resolve_enter(policy: str, role: str) -> str:
    if policy in ENTERS:
        return policy
    return {"accent": "stamp", "support": "fade"}.get(role, "draw")


def assign_enters(els, policy: str):
    """混合进场：主视觉手写、配角渐显、标题盖章；整板同一种时从第二个起换另两种。"""
    modes = [resolve_enter(policy, str(el.get("role") or "support")) for el in els]
    if policy in ENTERS or len(modes) < 2 or len(set(modes)) > 1:
        return modes
    rest = [m for m in ("stamp", "fade", "draw") if m != modes[0]]
    for i in range(1, len(modes)):
        modes[i] = rest[(i - 1) % len(rest)]
    return modes


def entrance_ms(mode, role, area, mean_area, speed, window):
    window = max(300, int(window))
    speed = max(0.25, float(speed))
    if mode == "stamp":
        natural, floor = 760 / max(0.55, speed), 400
    elif mode == "fade":
        natural, floor = 1080 / max(0.55, speed), 480
    else:
        natural = ROLE_BASE_MS.get(role, 2400) * float(np.clip((area / mean_area) ** 0.5, 0.7, 1.5)) / speed
        floor = 600
    return int(round(min(window, max(min(floor, window), natural))))


def element_crop(box, W, H):
    x, y, w, h = box
    x0, y0 = max(0, x - CROP_PAD), max(0, y - CROP_PAD)
    x1, y1 = min(W, x + w + CROP_PAD), min(H, y + h + CROP_PAD)
    return x0, y0, x1, y1


def transform_of(crop, W, H) -> dict:
    """裁出的图层放回画布原位：宿主按源宽高比居中适配后再缩放 / 平移。"""
    x0, y0, x1, y1 = crop
    cw, ch = x1 - x0, y1 - y0
    fit = min(W / cw, H / ch)
    return {"x": round((x0 + cw / 2) / W - 0.5, 6), "y": round((y0 + ch / 2) / H - 0.5, 6),
            "scale": round(1.0 / fit, 6), "scaleX": 1.0, "scaleY": 1.0, "opacity": 1.0}


# ── 书写：记录每个像素的出现时刻 ─────────────────────────────────────


class Capture(rsw.RegionStreamRenderer):
    """所有帧都经 _write_frame：不编码，只记录元素框内的 alpha 演进与笔尖。"""

    crop = (0, 0, 1, 1)
    tip_origin = (0, 0)
    budget = None
    tone = "chalk"
    samples_at: tuple = ()

    def setup(self, crop, budget, tone, samples_at):
        self.crop, self.budget, self.tone = crop, budget, tone
        x0, y0, x1, y1 = crop
        shape = (y1 - y0, x1 - x0)
        self.first = np.full((len(LEVELS),) + shape, -1, np.int32)
        self.tips: list = []
        self.count = 0
        self.samples_at = samples_at
        self.samples: dict[int, np.ndarray] = {}
        self.last = None
        self._pending_tip = None
        # The normal reveal path only keeps sparse verification samples. A
        # failed verification can opt into a second render and retain every
        # cropped RGBA frame for the host's packed-alpha encoder.
        self.capture_frames = False
        self.raw_frames: list[np.ndarray] = []

    def _snapshot_with_tip(self, px, py):
        self._last_tip = (int(px), int(py))
        self._pending_tip = self._last_tip
        return self.drawn.astype(np.uint8)

    def _reveal_ink_segment(self, a, b, allowed):
        """Same pixels as the library version, computed only around the
        segment instead of over the whole canvas (the bake's hot loop)."""
        radius = sr.reveal_radius(self.ink_halfwidth, a, b, self.cfg)
        pad = radius + 2
        x0, x1 = max(0, min(a[0], b[0]) - pad), min(self.out_w, max(a[0], b[0]) + pad + 1)
        y0, y1 = max(0, min(a[1], b[1]) - pad), min(self.out_h, max(a[1], b[1]) + pad + 1)
        if x1 <= x0 or y1 <= y0:
            return
        seg = np.zeros((y1 - y0, x1 - x0), dtype=np.uint8)
        cv2.line(seg, (a[0] - x0, a[1] - y0), (b[0] - x0, b[1] - y0), 255,
                 thickness=radius * 2 + 1, lineType=cv2.LINE_AA)
        window = np.s_[y0:y1, x0:x1]
        revealed = (seg > 0) & self.ink_pixels[window] & allowed[window]
        self.drawn[window][revealed] = self.ink_paint[window][revealed]

    # 上色不让手来回扫：线稿写完后颜色在本元素区域内直接淡入
    def _wash_contour(self, writer, frames, allowed):
        self._fade_color(writer, frames, allowed)

    def _wash_brush(self, writer, frames, centers, allowed):
        self._fade_color(writer, frames, allowed)

    def _fade_color(self, writer, frames, allowed):
        ys, xs = np.where(allowed)
        if ys.size == 0:
            return
        y0, y1, x0, x1 = ys.min(), ys.max() + 1, xs.min(), xs.max() + 1
        mask = allowed[y0:y1, x0:x1]
        view = self.drawn[y0:y1, x0:x1]
        base = (self.last if self.last is not None else self.drawn)[y0:y1, x0:x1][mask].astype(np.float32)
        target = self.color_img[y0:y1, x0:x1][mask].astype(np.float32)
        for fi in range(max(0, frames)):
            view[mask] = base + (target - base) * ((fi + 1) / frames)
            self._write_frame(writer, self.drawn.astype(np.uint8))
        view[mask] = target

    def pad(self):
        if self.budget is None:
            return
        snap = self.drawn.astype(np.uint8)
        while self.count < self.budget:
            self._write_frame(None, snap)

    def _write_frame(self, writer, snap):
        tip, self._pending_tip = self._pending_tip, None
        if self.budget is not None and self.count >= self.budget:
            return
        frame = snap if snap.dtype == np.uint8 else snap.astype(np.uint8)
        self.last = frame
        x0, y0, x1, y1 = self.crop
        if self.capture_frames:
            self.raw_frames.append(frame[y0:y1, x0:x1].copy())
        # Opacity only (to_straight_rgba's alpha), without the colour work.
        alpha = np.minimum(frame[y0:y1, x0:x1].max(axis=2).astype(np.float32) / 170.0, 1.0)
        # Keep the same first-crossing semantics without materializing a
        # 9×H×W ``crossed`` temporary for every frame. Once a level is filled
        # it is skipped for all later frames; the common tail therefore scans
        # only the few thresholds that still have unseen pixels.
        for level_index, level in enumerate(LEVELS):
            unseen = self.first[level_index] < 0
            if unseen.any():
                self.first[level_index][unseen & (alpha >= level)] = self.count
        if self.count in self.samples_at:
            self.samples[self.count] = alpha.copy()
        self.tips.append(None if tip is None else (tip[0] + self.tip_origin[0] - x0, tip[1] + self.tip_origin[1] - y0))
        self.count += 1


def renderer_for(cfg, single, ann, ann_dir, crop, W, H):
    """单元素在带边距的局部画布里书写；对齐网格与阅读带周期，笔迹与整幅一致。"""
    align = math.lcm(cfg.grid_edge, 24)
    margin = cfg.ink_reveal_max_radius
    x0, y0, x1, y1 = crop
    left = max(0, ((x0 - margin) // align) * align)
    top = max(0, ((y0 - margin) // align) * align)
    right = min(W, math.ceil((x1 + margin) / align) * align)
    bottom = min(H, math.ceil((y1 + margin) / align) * align)
    local = single[top:bottom, left:right]
    local_ann = copy.deepcopy(ann)
    local_ann["canvas"] = {"width": right - left, "height": bottom - top}
    for e in local_ann["elements"]:
        e["region"]["x"] -= left
        e["region"]["y"] -= top
    local_cfg = replace(cfg, cap_long_edge=max(right - left, bottom - top))
    r = Capture(local, local_ann, local_cfg, None, True, ann_dir=ann_dir)
    if (r.out_w, r.out_h) != (right - left, bottom - top):
        # 局部画布必须 1:1，否则时间图与定格画面对不上
        raise RuntimeError(f"书写画布尺寸 {r.out_w}x{r.out_h} 与 {right - left}x{bottom - top} 不符")
    return r, (x0 - left, y0 - top, x1 - left, y1 - top), (left, top)


def reveal_maps(r: Capture, fps: int, duration: float):
    """各档位的首次越过帧 → 每像素的完全显现时刻 t 与显现时长 s（秒）。"""
    _, final = to_straight_rgba(r.last[r_crop(r)], r.tone)
    present = final > 0.02
    n_frames = max(1, r.count)
    def level_time(fraction):
        target = final * fraction
        idx = np.clip(np.searchsorted(LEVELS, target, side="right") - 1, 0, len(LEVELS) - 1)
        f = np.take_along_axis(r.first, idx[None, ...], axis=0)[0].astype(np.float32)
        f[f < 0] = n_frames - 1
        return f
    t_full = (level_time(0.9) + 1) / fps
    t_start = level_time(0.12) / fps
    soft = np.clip(t_full - t_start, 1.0 / fps, SOFT_MAX)
    t_full = np.clip(t_full, 0, duration)
    return final, present, t_full, soft


def r_crop(r: Capture):
    x0, y0, x1, y1 = r.crop
    return np.s_[y0:y1, x0:x1]


def compose_alpha(final, present, t_full, soft, time):
    vis = smoothstep((time - (t_full - soft)) / np.maximum(soft, 1e-6))
    vis[time >= t_full] = 1.0
    return np.where(present, final * vis, 0.0)


def verify(r: Capture, final, present, t_full, soft, fps) -> tuple[bool, float]:
    """抽样帧：图集合成 vs 渲染器真实 alpha。只在元素出现过的像素上统计。"""
    if not r.samples:
        return True, 0.0
    region = present
    if not region.any():
        return True, 0.0
    worst = 0.0
    for index, actual in r.samples.items():
        composed = compose_alpha(final, present, t_full, soft, (index + 0.5) / fps)
        err = float(np.abs(composed - actual)[region].mean())
        worst = max(worst, err)
    return worst <= 0.035, worst


def write_reveal(path: Path, color_bgr: np.ndarray, final, present, t_full, soft, duration, hand):
    """todo-reveal/1：左半定格画面（直通 RGBA），右半 R/G=16 位出现时刻，B=显现时长，A=参与。"""
    h, w = final.shape
    picture = np.dstack([color_bgr[..., 2], color_bgr[..., 1], color_bgr[..., 0], np.clip(final, 0, 1) * 255.0])
    t16 = np.clip(np.round(t_full / duration * 65535.0), 0, 65535).astype(np.uint32)
    timing = np.zeros((h, w, 4), np.uint8)
    timing[..., 0] = (t16 >> 8).astype(np.uint8)
    timing[..., 1] = (t16 & 255).astype(np.uint8)
    timing[..., 2] = np.clip(np.round(soft / SOFT_MAX * 255.0), 0, 255).astype(np.uint8)
    timing[..., 3] = np.where(present, 255, 0).astype(np.uint8)
    atlas = np.concatenate([np.clip(picture, 0, 255).astype(np.uint8), timing], axis=1)
    meta = {"spec": REVEAL_SPEC, "width": w, "height": h, "duration": round(float(duration), 4), "softMax": SOFT_MAX}
    if hand:
        meta["hand"] = hand
    info = PngImagePlugin.PngInfo()
    info.add_itxt("todo-reveal", json.dumps(meta, ensure_ascii=False, separators=(",", ":")))
    path.parent.mkdir(parents=True, exist_ok=True)
    Image.fromarray(atlas, "RGBA").save(path, "PNG", pnginfo=info, optimize=False, compress_level=6)


def hand_path(tips, crop_w, crop_h, fps):
    idx = [i for i, t in enumerate(tips) if t]
    if not idx:
        return []
    step = max(1, round(fps / TIP_RATE))
    keep = sorted(set(list(range(idx[0], idx[-1] + 1, step)) + [idx[-1]]))
    out, last = [], None
    for i in keep:
        last = tips[i] or last
        if last is None:
            continue
        out.append([round(i / fps, 3), round(last[0] / crop_w, 4), round(last[1] / crop_h, 4)])
    return out


def ensure_hand(out_dir: Path, tip) -> dict | None:
    """粉笔手图（直通 RGBA），所有手写元素共用；返回笔尖锚点（图内 0~1）与像素高度。"""
    if tip is None:
        return None
    path = out_dir / HAND_FILE
    if not path.is_file():
        rgba = np.dstack([tip.hand[..., 2], tip.hand[..., 1], tip.hand[..., 0], np.clip(tip.mask * 255.0, 0, 255)])
        path.parent.mkdir(parents=True, exist_ok=True)
        Image.fromarray(rgba.astype(np.uint8), "RGBA").save(path, "PNG")
    return {"anchor": [round(tip.tip_px / max(1, tip.w - 1), 4), round(tip.tip_py / max(1, tip.h - 1), 4)],
            "pixels": int(tip.h)}


# Stamp dust is a small, deterministic port of the old renderer's particle
# recipe. It is emitted as a linked packed-alpha item so the main image keeps
# its normal editable transform and the dust follows it on the timeline.
_DUST_DISC_CACHE: dict[int, np.ndarray] = {}


def _dust_disc(radius: float) -> np.ndarray:
    key = max(2, int(round(radius * 2)))
    cached = _DUST_DISC_CACHE.get(key)
    if cached is not None:
        return cached
    r = max(1, key // 2)
    yy, xx = np.mgrid[-r:r + 1, -r:r + 1]
    disc = np.clip(1.0 - np.hypot(xx, yy) / max(radius, 0.5), 0.0, 1.0) ** 1.7
    _DUST_DISC_CACHE[key] = disc
    return disc


def _splat(acc: np.ndarray, disc: np.ndarray, x: float, y: float, amount: float) -> None:
    if amount <= 0.004:
        return
    h, w = acc.shape
    dh, dw = disc.shape
    cx, cy = int(round(x)), int(round(y))
    x0, y0 = cx - dw // 2, cy - dh // 2
    x1, y1 = x0 + dw, y0 + dh
    if x1 <= 0 or y1 <= 0 or x0 >= w or y0 >= h:
        return
    sx0, sy0 = max(0, -x0), max(0, -y0)
    dx0, dy0 = max(0, x0), max(0, y0)
    dx1, dy1 = min(w, x1), min(h, y1)
    acc[dy0:dy1, dx0:dx1] += disc[sy0:sy0 + dy1 - dy0, sx0:sx0 + dx1 - dx0] * amount


def _make_dust(alpha: np.ndarray, seed: int, tone: str) -> dict:
    h, w = alpha.shape
    ink = (alpha > 0.22).astype(np.uint8)
    rim = (cv2.dilate(ink, cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (5, 5))) > 0) & (
        cv2.erode(ink, cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (3, 3))) == 0
    )
    ys, xs = np.where(rim)
    rng = np.random.default_rng(seed & 0xFFFFFFFF)
    if xs.size < 8:
        yy, xx = np.mgrid[0:h:6, 0:w:6]
        ys, xs = yy.ravel(), xx.ravel()
    n = int(np.clip(float(ink.sum()) / 42.0, 70, 150))
    weight = (ys - ys.min() + 1.0) ** 1.35
    weight /= weight.sum()
    pick = rng.choice(xs.size, size=n, replace=True, p=weight)
    px, py = xs[pick].astype(np.float64), ys[pick].astype(np.float64)
    cx, cy = float(np.average(xs, weights=weight)), float(np.average(ys, weights=weight))
    dx, dy = px - cx, py - cy
    norm = np.hypot(dx, dy) + 1e-3
    scale = max(0.25, max(alpha.shape) * 0.45 / 140.0)
    vx = dx / norm * rng.uniform(50, 210, n) * scale + rng.normal(0, 18, n) * scale
    vy = dy / norm * rng.uniform(10, 70, n) * scale - rng.uniform(140, 340, n) * scale
    life = rng.uniform(0.38, 0.78, n)
    radius = rng.uniform(1.3, 4.2, n) * scale
    peak = rng.uniform(0.45, 0.95, n)
    puffs = max(10, n // 6)
    puff_i = rng.choice(n, size=puffs, replace=False)
    radius[puff_i] = rng.uniform(9.0, 18.0, puffs) * scale
    peak[puff_i] = rng.uniform(0.28, 0.5, puffs)
    vy[puff_i] -= rng.uniform(30, 110, puffs) * scale
    return {"x": px, "y": py, "vx": vx, "vy": vy, "life": life,
            "radius": radius, "peak": peak,
            "rgb": np.array([210, 228, 236], np.float32) if tone != "ink" else np.array([42, 44, 48], np.float32),
            "gravity": 340.0 * scale}


def _dust_alpha(dust: dict, age: float, shape: tuple[int, int]) -> np.ndarray:
    h, w = shape
    acc = np.zeros((h, w), np.float32)
    alive = age < dust["life"]
    if not np.any(alive) or age < 0:
        return acc
    t = np.full(dust["life"].shape, age, np.float64)
    x = dust["x"] + dust["vx"] * t
    y = dust["y"] + dust["vy"] * t + 0.5 * float(dust["gravity"]) * t * t
    fade = np.clip(1.0 - t / dust["life"], 0, 1) ** 1.35
    grow = 1.0 + 1.5 * np.clip(t / dust["life"], 0, 1)
    for i in np.flatnonzero(alive):
        _splat(acc, _dust_disc(float(dust["radius"][i] * grow[i])), float(x[i]), float(y[i]),
               float(dust["peak"][i] * fade[i]))
    return np.clip(acc, 0, 0.85)


def work_relative(ctx, path: Path) -> str:
    """Return a protocol path even when Windows paths use different `..` spellings."""
    return os.path.relpath(path, ctx.workdir).replace("\\", "/")


def stamp_dust(ctx, out_dir: Path, element_id: str, bgr: np.ndarray, tone: str,
               fps: int, duration: float, seed: int) -> str:
    color, alpha = to_straight_rgba(bgr, tone)
    frame_count = max(2, int(round(duration * fps)))
    frames_dir = out_dir / f"{element_id}.dust.frames"
    shutil.rmtree(frames_dir, ignore_errors=True)
    frames_dir.mkdir(parents=True, exist_ok=True)
    dust = _make_dust(alpha, seed, tone)
    ys, xs = np.where(alpha > 0.15)
    cx = float(xs.mean()) if xs.size else bgr.shape[1] / 2
    cy = float(ys.mean()) if ys.size else bgr.shape[0] / 2
    content_h = float((ys.max() - ys.min() + 1) if ys.size else bgr.shape[0])
    for index in range(frame_count):
        u = index / max(1, frame_count - 1)
        dust_u = np.clip((u - 0.36) / 0.64, 0.0, 1.0)
        dust_a = _dust_alpha(dust, dust_u * max(0.18, duration * 0.64), alpha.shape)
        rise = dust_u * content_h * 0.62
        cloud = content_h * (0.18 + dust_u * 0.42)
        amount = 0.55 * (1.0 - dust_u) ** 1.15
        _splat(dust_a, _dust_disc(cloud), cx, cy + content_h * 0.12 - rise, amount)
        _splat(dust_a, _dust_disc(cloud * 0.5), cx - content_h * 0.38, cy - rise * 0.65, amount * 0.75)
        _splat(dust_a, _dust_disc(cloud * 0.5), cx + content_h * 0.38, cy - rise * 0.65, amount * 0.75)
        dust_color = np.empty_like(color)
        dust_color[...] = dust["rgb"]
        save_png(frames_dir / f"{index + 1:06d}.png", bgra(dust_color, dust_a))
    relative = work_relative(ctx, frames_dir)
    encoded = ctx.call("encode", {"frames": relative, "fps": fps, "alpha": True})
    file = encoded.get("file") if isinstance(encoded, dict) else None
    if not isinstance(file, str) or not file:
        raise RunFailed(f"元素 {element_id} 的粉笔灰编码没有返回文件", "internal")
    return file


def packed_fallback(ctx, out_dir: Path, element_id: str, r: Capture, crop, tone: str,
                    fps: int, duration: float, work: Path) -> str:
    """Replay a failed reveal and let the host encode its exact RGBA frames.

    Reveal timing is a compact approximation and cannot represent an element
    that is erased or painted over. The fallback deliberately reuses the same
    renderer and local crop, then hands the PNG sequence to the supervised
    ``encode`` service so no FFmpeg process is started by the worker.
    """
    frames_dir = out_dir / f"{element_id}.frames"
    shutil.rmtree(frames_dir, ignore_errors=True)
    frames_dir.mkdir(parents=True, exist_ok=True)
    # Reinitialize the capture buffers before replaying. The first pass may
    # have kept only sparse samples and must not be mixed into this sequence.
    r.setup(crop, r.budget, tone, ())
    r.capture_frames = True
    r.render_to(work / "_fallback_unused.mp4", duration, draw_ahead_ms=0, lock_reveal=True)
    r.pad()
    if not r.raw_frames:
        raise RunFailed(f"元素 {element_id} 回退没有生成画面", "internal")
    for index, frame in enumerate(r.raw_frames, 1):
        color, alpha = to_straight_rgba(frame, tone)
        save_png(frames_dir / f"{index:06d}.png", bgra(color, alpha))
    relative = work_relative(ctx, frames_dir)
    encoded = ctx.call("encode", {"frames": relative, "fps": fps, "alpha": True})
    file = encoded.get("file") if isinstance(encoded, dict) else None
    if not isinstance(file, str) or not file:
        raise RunFailed(f"元素 {element_id} 的 packed-alpha 编码没有返回文件", "internal")
    return file


def config(params, W, H, fps):
    fill = str(params.get("fill") or "wipe")
    return sr.Config(
        fps=fps,
        cap_long_edge=max(W, H),
        ink_path_mode=str(params.get("stroke") or "skeleton"),
        color_fill="brush" if fill == "brush" else "contour-wipe",
        color_weight=0 if fill == "none" else 1,
        ink_weight=9,
        pause_mode="heavy",
        board_style="chalkboard",
        canvas_hex="#0b241c",
        ink_threshold=52,
        target_hand_height=max(120, int(round(min(W, H) * float(params.get("handSize") or 1.0)))),
    )


def shared_tip(cfg):
    hand_png = BLACKBOARD / "assets" / "hand-chalk.png"
    data = sr._load_hand(hand_png, cfg.target_hand_height) if hand_png.is_file() else None
    ax, ay = cfg.tip_anchor_x, cfg.tip_anchor_y
    if data is None:
        data = sr._procedural_tip(cfg.target_hand_height)
        ax, ay = 0.5, 0.70
    return sr.TipOverlay(data[0], data[1], tip_anchor_x=ax, tip_anchor_y=ay)


# ── 操作 ────────────────────────────────────────────────────────────


@worker.op("bake_board")
def bake_board(ctx):
    value = ctx.input if isinstance(ctx.input, dict) else {}
    if value.get("spec") != BAKE_SPEC:
        raise RunFailed(f"输入须为 {BAKE_SPEC}", "invalid_input")
    params = value.get("params") or {}
    canvas = value.get("canvas") or {}
    W, H = int(canvas.get("width", 1920)), int(canvas.get("height", 1080))
    fps = max(12, min(60, int(round(float(canvas.get("fps", 30))))))
    els = [e for e in value.get("elements") or [] if isinstance(e, dict)]
    if not 1 <= len(els) <= 8:
        raise RunFailed("每块黑板须有 1~8 个元素", "invalid_input")
    tone = str(params.get("tone") or "chalk")
    speed = float(params.get("drawSpeed") or 1.0)
    policy = str(params.get("enter") or "mix")
    with_hand = str(params.get("hand") or "chalk") != "none"
    board = str(value.get("board") or "board")
    out_dir = ctx.path(f"out/{board}")
    work = ctx.path(f"work/{board}")
    shutil.rmtree(work, ignore_errors=True)
    (work / "masks").mkdir(parents=True, exist_ok=True)

    ctx.progress("抠图与排版", 0.05)
    crops = [cut(ctx.path(str(e["file"]))) for e in els]
    _, placed, singles = place(els, crops, W, H, str(params.get("layout") or "auto"), work / "masks")
    modes = assign_enters(els, policy)
    areas = [p["area"] for p in placed]
    mean_area = (sum(areas) / len(areas)) or 1.0
    cfg = config(params, W, H, fps)
    tip = shared_tip(cfg) if with_hand else None
    hand = ensure_hand(out_dir, tip) if tip is not None else None

    items, fallbacks, worst = [], 0, 0.0
    for i, (el, p, single, mode) in enumerate(zip(els, placed, singles, modes)):
        ctx.check_cancelled()
        at = float(el.get("at") or 0.0)
        window_ms = max(300, int(round((float(el.get("windowEnd") or at + 1.0) - at) * 1000)))
        du = entrance_ms(mode, p["role"], p["area"], mean_area, speed, window_ms)
        crop = element_crop(p["box"], W, H)
        x0, y0, x1, y1 = crop
        key = str(el.get("key") or f"{board}/{el['id']}")
        # Static elements used to advertise a fixed one-second duration even
        # when their entrance animation was longer.  The host correctly
        # rejects an intro that exceeds the item duration, so keep the item
        # alive for at least the computed entrance window.
        item_duration = max(1.0, round(du / 1000.0, 4))
        item = {"key": key, "role": "visual", "lane": int(el.get("lane") or i + 1),
                "start": round(at, 3), "duration": item_duration, "transform": transform_of(crop, W, H)}
        if mode == "draw":
            ctx.progress(f"书写 {p['label'] or p['id']}", 0.1 + 0.85 * i / len(els))
            ann = {"sceneId": f"{board}_{p['id']}", "canvas": {"width": W, "height": H},
                   "sceneDurationMs": du + DRAW_TAIL_MS, "timingSource": "tts_timeline",
                   "elements": [{"id": p["id"], "label": p["label"], "sequence": 1, "type": "structure",
                                 "region": {"x": p["box"][0], "y": p["box"][1], "width": p["box"][2], "height": p["box"][3]},
                                 "reveal": {"direction": "left_to_right", "startMs": 0, "durationMs": du, "protectedRegions": []},
                                 "framed": True, "maskFile": f"masks/{p['id']}.png"}]}
            r, local_crop, origin = renderer_for(cfg, single, ann, work, crop, W, H)
            r.tip = tip
            budget = max(2, int(round((du + DRAW_TAIL_MS) * fps / 1000)))
            samples = tuple(int(round(budget * k / 9)) for k in range(1, 9))
            r.setup(local_crop, budget, tone, samples)
            r.tip_origin = (0, 0)
            r.render_to(work / "_unused.mp4", du + DRAW_TAIL_MS, draw_ahead_ms=0, lock_reveal=True)
            r.pad()
            duration = budget / fps
            final, present, t_full, soft = reveal_maps(r, fps, duration)
            ok, err = verify(r, final, present, t_full, soft, fps)
            worst = max(worst, err)
            if not ok:
                fallbacks += 1
                packed = packed_fallback(ctx, out_dir, str(p["id"]), r, r.crop, tone,
                                         fps, duration, work)
                item["source"] = {"type": "file", "file": packed}
                item["duration"] = round(duration, 4)
            else:
                color, _ = to_straight_rgba(r.last[r_crop(r)], tone)
                meta_hand = None
                if hand and (path := hand_path(r.tips, x1 - x0, y1 - y0, fps)):
                    meta_hand = {"file": HAND_FILE, "anchor": hand["anchor"],
                                 "height": round(hand["pixels"] / (y1 - y0), 4), "path": path}
                name = f"{p['id']}.reveal.png"
                write_reveal(out_dir / name, color, final, present, t_full, soft, duration, meta_hand)
                item["source"] = {"type": "file", "file": f"out/{board}/{name}"}
        else:
            name = f"{p['id']}.png"
            color, alpha = to_straight_rgba(single[y0:y1, x0:x1], tone)
            save_png(out_dir / name, bgra(color, alpha))
            item["source"] = {"type": "file", "file": f"out/{board}/{name}"}
            item["intro"] = {"kind": "stamp" if mode == "stamp" else "fade",
                              "duration": min(round(du / 1000.0, 3), item_duration)}
            if mode == "stamp":
                dust_file = stamp_dust(
                    ctx,
                    out_dir,
                    str(p["id"]),
                    single[y0:y1, x0:x1],
                    tone,
                    fps,
                    max(0.1, du / 1000.0),
                    zlib.crc32(key.encode("utf-8")),
                )
                items.append({
                    "key": f"{key}/dust",
                    "rev": format(zlib.crc32(f"{key}/dust".encode("utf-8")), "08x"),
                    "role": "visual",
                    "lane": min(15, int(item["lane"]) + 1),
                    "start": round(at, 3),
                    "duration": round(max(0.1, du / 1000.0), 4),
                    "source": {"type": "file", "file": dust_file},
                    "link": key,
                })
        item["rev"] = format(zlib.crc32(json.dumps([value, i], sort_keys=True, ensure_ascii=False).encode()), "08x")
        items.append(item)
    shutil.rmtree(work, ignore_errors=True)
    ctx.emit(items)
    ctx.progress("完成", 1.0)
    return {"board": board, "items": len(items), "fallbacks": fallbacks, "revealError": round(worst, 4),
            "modes": modes}


@worker.op("plate")
def plate(ctx):
    """整片黑板底：黑板报底图按画布 cover 裁切，放在最底层轨道。"""
    value = ctx.input if isinstance(ctx.input, dict) else {}
    canvas = value.get("canvas") or {}
    W, H = int(canvas.get("width", 1920)), int(canvas.get("height", 1080))
    src = BLACKBOARD / "assets" / "chalkboard-bg-black.png"
    img = load_bgr(src)
    h, w = img.shape[:2]
    s = max(W / w, H / h)
    img = cv2.resize(img, (max(W, round(w * s)), max(H, round(h * s))), interpolation=cv2.INTER_AREA if s < 1 else cv2.INTER_CUBIC)
    y0, x0 = (img.shape[0] - H) // 2, (img.shape[1] - W) // 2
    name = f"out/plate_{W}x{H}.png"
    save_png(ctx.path(name), img[y0:y0 + H, x0:x0 + W])
    ctx.emit([{"key": str(value.get("key") or "plate"), "rev": f"{W}x{H}", "role": "visual", "lane": 0,
               "start": 0.0, "duration": 1.0, "source": {"type": "file", "file": name}}])
    return {"width": W, "height": H}


if __name__ == "__main__":
    worker.serve()
