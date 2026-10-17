#!/usr/bin/env python3
"""
SRT 白板动画 - 整合渲染器（mask 编排 + stream 画法）

把一张线稿图 + 同名 annotation.json 渲染成白板手绘动画：
  - 编排沿用 whiteboard-mask-animation：按 sequence/startMs 顺序逐区域揭示，
    每个区域的可作画范围 = 矩形 region 扣除「后续区域 + protectedRegions」，
    未开始的区域因掩码限制不会提前露线（mask 的核心不变量）。
  - 画法换成 whiteboard-stream-animation：每个区域在自己的允许掩码内，
    沿骨架/网格笔迹连续落墨（起笔 ink → 添彩 color），笔尖跟随真实笔迹，
    所有区域共享同一张持久画布，已画完的区域保留在画布上。

与 mask 的矩形擦除揭示不同：这里是「笔尖沿线滑行、边走边落墨」的连贯笔迹。
输出末行打印 OUTPUT=<路径>，便于上层捕获。

用法：
  <ENV_PY> render_stream_whiteboard.py <图片> <标注json> <输出mp4> [手部素材png]
  可选参数见 --help（--ink-path / --color-fill / --pause / --total-ms 等）。
  --total-ms 缺省时用标注里的 sceneDurationMs。
"""
from __future__ import annotations

import argparse
import datetime
import json
import math
import sys
from pathlib import Path

import cv2
import numpy as np

# 复用 stream 渲染器的全部构件（同目录）
_SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(_SCRIPT_DIR))
import stream_render as sr  # noqa: E402

DEFAULT_HAND = _SCRIPT_DIR.parent / "assets" / "hand-chalk.png"
# 硬锁：书写底板用内置黑砂黑板，禁止纯色墨绿 #0b241c
CHALK_BG = _SCRIPT_DIR.parent / "assets" / "chalkboard-bg-black.png"
CHALK_BG_FALLBACK = _SCRIPT_DIR.parent / "assets" / "chalkboard-bg.png"
DRAW_AHEAD_MS = 2000  # 书写须在口播结束前这么多毫秒画完，剩下对着完成画面听
MIN_INK_PX = 200  # 重叠框只剩残渣时不空手比划，时间匀给真正有墨的区


def _load_chalk_board(h: int, w: int) -> np.ndarray:
    """返回 HxWx3 uint8 黑板底（优先黑砂底）。"""
    from blackbg_element_hand import make_board

    if CHALK_BG.is_file():
        return make_board(h, w, CHALK_BG)
    if CHALK_BG_FALLBACK.is_file():
        print("  [warn] 缺 chalkboard-bg-black.png，临时用 chalkboard-bg.png")
        return make_board(h, w, CHALK_BG_FALLBACK)
    print("  [warn] 缺黑板底图，回退颗粒黑板")
    return make_board(h, w, None)


# ──────────────────────────────────────────────────────────────
# 区域几何：把标注画布坐标缩放到输出尺寸
# ──────────────────────────────────────────────────────────────
def _scaled_rect(region: dict, sx: float, sy: float, out_w: int, out_h: int) -> tuple[int, int, int, int]:
    x0 = int(round(region["x"] * sx))
    y0 = int(round(region["y"] * sy))
    x1 = int(round((region["x"] + region["width"]) * sx))
    y1 = int(round((region["y"] + region["height"]) * sy))
    x0 = max(0, min(out_w, x0))
    x1 = max(0, min(out_w, x1))
    y0 = max(0, min(out_h, y0))
    y1 = max(0, min(out_h, y1))
    return x0, y0, x1, y1


def _frame_progress_indices(n_steps: int, target_frames: int) -> list[int]:
    """把 n_steps 个笔尖位置均匀映射到 target_frames 帧。"""
    if n_steps == 0 or target_frames <= 0:
        return []
    if target_frames == 1:
        return [n_steps - 1]
    return [round(f * (n_steps - 1) / (target_frames - 1)) for f in range(target_frames)]


def _chalk_roughen_alpha(alpha: np.ndarray) -> np.ndarray:
    """钢笔掩码边缘「毛毛」：宽啃边 + 须状外延 + 粉尘。"""
    a = alpha.astype(np.uint8) * 255
    if a.size == 0:
        return alpha
    h, w = a.shape
    k3 = np.ones((3, 3), np.uint8)
    k7 = np.ones((7, 7), np.uint8)
    k9 = np.ones((9, 9), np.uint8)
    dil = cv2.dilate(a, k7, iterations=1)
    dil_far = cv2.dilate(a, k9, iterations=1)
    ero = cv2.erode(a, k3, iterations=1)
    ero2 = cv2.erode(a, k7, iterations=1)
    rim = (a > 0) & (ero == 0)          # 紧贴边缘
    band = (a > 0) & (ero2 == 0)        # 更宽一圈内缘
    near = (a == 0) & (dil > 0)
    far = (a == 0) & (dil_far > 0) & (dil == 0)
    yy, xx = np.mgrid[0:h, 0:w]
    noise = ((xx * 374761393 + yy * 668265263) & 0xFFFFFFFF) / 4294967296.0
    noise2 = ((xx * 982451653 + yy * 2654435761) & 0xFFFFFFFF) / 4294967296.0
    out = a.copy()
    # 内缘狠啃 → 毛糙过渡
    out[band & (noise < 0.42)] = 0
    out[rim & (noise < 0.62)] = 0
    out[rim & (noise >= 0.62) & (noise < 0.78)] = 0
    # 外侧短须：按噪声方向戳 2~6px
    ys, xs = np.where(near & (noise2 < 0.35))
    for x, y in zip(xs.tolist(), ys.tolist()):
        ang = noise2[y, x] * math.pi * 2
        length = 2 + int(noise[y, x] * 6)
        for k in range(1, length + 1):
            xx2 = int(round(x + math.cos(ang) * k))
            yy2 = int(round(y + math.sin(ang) * k))
            if xx2 < 0 or yy2 < 0 or xx2 >= w or yy2 >= h:
                break
            if a[yy2, xx2] > 0 and k > 1:
                break
            out[yy2, xx2] = 255
    # 更远一点的粉笔末
    out[far & (noise > 0.93)] = 255
    out[near & (noise > 0.86)] = 255
    return out > 8


# ──────────────────────────────────────────────────────────────
# 每区域的 stream 笔迹渲染，写入共享持久画布
# ──────────────────────────────────────────────────────────────
class RegionStreamRenderer:
    """持有整段渲染的共享状态；逐区域把 stream 笔迹画进同一张画布。"""

    def __init__(self, image_bgr: np.ndarray, annotation: dict, cfg: sr.Config,
                 hand_png: Path | None, bare_tip: bool,
                 ann_dir: Path | None = None) -> None:
        self.cfg = cfg
        self.ann = annotation
        self.ann_dir = Path(ann_dir) if ann_dir else None
        self._shape_cache: dict[str, np.ndarray | None] = {}

        # 输出尺寸：长边限到 cap，对齐到 grid_edge 的偶数倍（编码要求偶数）
        h0, w0 = image_bgr.shape[:2]
        scale = cfg.cap_long_edge / max(h0, w0)
        align = cfg.grid_edge if cfg.grid_edge % 2 == 0 else cfg.grid_edge * 2
        w = max(align, (int(round(w0 * scale)) // align) * align)
        h = max(align, (int(round(h0 * scale)) // align) * align)
        self.out_w, self.out_h = w, h

        # 黑板底图（砂感），非纯色 hex
        if cfg.board_style == "paper":
            self.canvas_bgr = np.full(
                (self.out_h, self.out_w, 3), sr._hex_to_bgr(cfg.canvas_hex), dtype=np.uint8
            )
        else:
            self.canvas_bgr = _load_chalk_board(self.out_h, self.out_w)

        # 标注画布坐标 → 输出坐标的缩放比
        cw = annotation["canvas"]["width"]
        ch = annotation["canvas"]["height"]
        self.sx = self.out_w / cw
        self.sy = self.out_h / ch

        self.color_img = cv2.resize(image_bgr, (self.out_w, self.out_h), interpolation=cv2.INTER_AREA)
        # 灰度取自染色前的原图，墨迹判定不受背景替换影响。
        gray = cv2.cvtColor(self.color_img, cv2.COLOR_BGR2GRAY)

        # 背景染成黑板底。必须早于 _ink_layers：起笔要落的就是这份像素。
        if cfg.match_bg:
            self._match_original_background()

        self.thresh_map, self.ink_pixels, self.ink_paint = sr._ink_layers(self.color_img, gray, cfg)
        # 非墨区也铺黑板底纹理（_ink_layers 默认铺纯色）
        if cfg.board_style != "paper":
            self.ink_paint[~self.ink_pixels] = self.canvas_bgr[~self.ink_pixels].astype(np.float32)
        self.ink_halfwidth = sr.stroke_halfwidth(self.ink_pixels)
        self.grid_blocks = sr._to_grid_blocks(self.thresh_map, cfg.grid_edge)
        active_thr = cfg.ink_threshold if cfg.board_style == "paper" else 10
        self.active_all = sr._active_mask(self.thresh_map, cfg.grid_edge, active_thr)

        # 共享持久画布
        self.drawn = np.empty((self.out_h, self.out_w, 3), dtype=np.float32)
        self.drawn[...] = self.canvas_bgr.astype(np.float32)

        # 笔尖覆盖
        self.tip: sr.TipOverlay | None = None
        self.tip_track: list[tuple[int, int] | None] = []
        self._last_tip: tuple[int, int] | None = None
        self._record_tips = False
        if not bare_tip:
            hand_data = sr._load_hand(hand_png, cfg.target_hand_height) if hand_png else None
            ax, ay = cfg.tip_anchor_x, cfg.tip_anchor_y
            if hand_data is None:
                hand_data = sr._procedural_tip(cfg.target_hand_height)
                ax, ay = 0.5, 0.70
            self.tip = sr.TipOverlay(hand_data[0], hand_data[1], tip_anchor_x=ax, tip_anchor_y=ay)

    # 采样原图四角，把接近背景色的像素替换为黑板底纹理
    def _match_original_background(self) -> None:
        img = self.color_img
        h, w = img.shape[:2]
        margin = max(3, min(h, w) // 50)
        samples = [img[:margin, :margin], img[:margin, -margin:],
                   img[-margin:, :margin], img[-margin:, -margin:]]
        bg = np.median(np.concatenate([s.reshape(-1, 3) for s in samples]), axis=0)
        diff = np.abs(img.astype(np.int16) - bg.astype(np.int16)).sum(axis=2)
        mask = diff < self.cfg.match_bg_threshold
        img[mask] = self.canvas_bgr[mask]

    def _cell_center(self, cell: tuple[int, int]) -> tuple[int, int]:
        r, c = cell
        e = self.cfg.grid_edge
        return (c * e + e // 2, r * e + e // 2)

    def _snapshot_with_tip(self, px: int, py: int) -> np.ndarray:
        self._last_tip = (int(px), int(py))
        snap = self.drawn.astype(np.uint8)
        if self.tip is not None:
            self.tip.stamp(snap, px, py)
        return snap

    def _write_frame(self, writer, snap: np.ndarray) -> None:
        if self._record_tips:
            self.tip_track.append(self._last_tip)
        writer.write(snap)

    # ── 钢笔框选 PNG（不规则）→ 全图画布布尔掩码 ──
    def _shape_mask(self, element: dict) -> np.ndarray | None:
        eid = str(element.get("id") or "")
        if eid in self._shape_cache:
            return self._shape_cache[eid]
        rel = element.get("maskFile") or (f"masks/{eid}.png" if eid else "")
        if not rel or not self.ann_dir:
            self._shape_cache[eid] = None
            return None
        path = (self.ann_dir / str(rel)).resolve()
        try:
            path.relative_to(self.ann_dir.resolve())
        except ValueError:
            self._shape_cache[eid] = None
            return None
        if not path.is_file():
            self._shape_cache[eid] = None
            return None
        # Windows 中文路径下 cv2.imread 会失败，改用 imdecode
        try:
            raw = np.fromfile(str(path), dtype=np.uint8)
            rgba = cv2.imdecode(raw, cv2.IMREAD_UNCHANGED)
        except Exception:
            rgba = None
        if rgba is None:
            self._shape_cache[eid] = None
            return None
        if rgba.ndim == 3 and rgba.shape[2] >= 4:
            alpha = rgba[:, :, 3] > 8
        else:
            gray = rgba if rgba.ndim == 2 else cv2.cvtColor(rgba, cv2.COLOR_BGR2GRAY)
            alpha = gray > 8
        x0, y0, x1, y1 = _scaled_rect(element["region"], self.sx, self.sy, self.out_w, self.out_h)
        tw, th = max(0, x1 - x0), max(0, y1 - y0)
        if tw <= 0 or th <= 0:
            self._shape_cache[eid] = None
            return None
        if alpha.shape[0] != th or alpha.shape[1] != tw:
            alpha = cv2.resize(alpha.astype(np.uint8), (tw, th), interpolation=cv2.INTER_NEAREST).astype(bool)
        # 钢笔掩码边缘粉笔化：咬边+粉尘，避免光滑接缝假边
        alpha = _chalk_roughen_alpha(alpha)
        full = np.zeros((self.out_h, self.out_w), dtype=bool)
        full[y0:y1, x0:x1] = alpha
        self._shape_cache[eid] = full
        return full

    # ── 单区域的允许掩码：优先钢笔不规则形状；否则矩形 - 后续区 - protectedRegions ──
    def _allowed_mask(self, element: dict, later_elements: list[dict]) -> np.ndarray:
        shape = self._shape_mask(element)
        if shape is not None:
            # 用户框选形状为准：只扣掉后续模块的真实形状，绝不拿大矩形互切（避免接缝高亮）
            mask = shape.copy()
            for later in later_elements:
                ls = self._shape_mask(later)
                if ls is not None:
                    mask &= ~ls
                else:
                    lx0, ly0, lx1, ly1 = _scaled_rect(later["region"], self.sx, self.sy, self.out_w, self.out_h)
                    mask[ly0:ly1, lx0:lx1] = False
            return mask

        mask = np.zeros((self.out_h, self.out_w), dtype=bool)
        x0, y0, x1, y1 = _scaled_rect(element["region"], self.sx, self.sy, self.out_w, self.out_h)
        mask[y0:y1, x0:x1] = True
        for later in later_elements:
            ls = self._shape_mask(later)
            if ls is not None:
                mask &= ~ls
            else:
                lx0, ly0, lx1, ly1 = _scaled_rect(later["region"], self.sx, self.sy, self.out_w, self.out_h)
                mask[ly0:ly1, lx0:lx1] = False
        for prot in element.get("reveal", {}).get("protectedRegions", []):
            px0, py0, px1, py1 = _scaled_rect(prot, self.sx, self.sy, self.out_w, self.out_h)
            mask[py0:py1, px0:px1] = False
        # 矩形切到字/图标中间会「裂开」：只把近邻连通墨迹并入，避免虚线/箭头吞掉远处元素
        expanded = self._expand_mask_to_ink_components(mask)
        # 扩展后再扣一次：后续区 / 保护区仍归后画的元素
        for later in later_elements:
            ls = self._shape_mask(later)
            if ls is not None:
                expanded &= ~ls
            else:
                lx0, ly0, lx1, ly1 = _scaled_rect(later["region"], self.sx, self.sy, self.out_w, self.out_h)
                expanded[ly0:ly1, lx0:lx1] = False
        for prot in element.get("reveal", {}).get("protectedRegions", []):
            px0, py0, px1, py1 = _scaled_rect(prot, self.sx, self.sy, self.out_w, self.out_h)
            expanded[py0:py1, px0:px1] = False
        return expanded

    def _expand_mask_to_ink_components(self, seed: np.ndarray) -> np.ndarray:
        """seed 碰到的墨迹连通域中，仅并入「靠近 seed」的像素，避免跨元素吞墨。"""
        ink = self.ink_pixels
        if not (ink & seed).any():
            return seed
        n, labels = cv2.connectedComponents(ink.astype(np.uint8), connectivity=8)
        touch = np.unique(labels[seed & ink])
        touch = touch[touch > 0]
        if touch.size == 0:
            return seed
        # 只允许延伸到 seed 外一圈，防止虚线把远处主体并进来
        pad = max(16, min(self.out_w, self.out_h) // 32)
        k = 2 * pad + 1
        near = cv2.dilate(seed.astype(np.uint8), np.ones((k, k), np.uint8), iterations=1).astype(bool)
        expanded = seed.copy()
        for lab in touch:
            expanded |= (labels == lab) & near
        return expanded

    # ── 区域内笔迹路径 ──
    def _region_grid_streams(self, allowed: np.ndarray) -> list[list[tuple[int, int]]]:
        """网格模式：返回多条墨流（每条=一块视觉元素），勿提前 flatten。"""
        allowed_u8 = allowed.astype(np.uint8)
        allowed_cell = sr._to_grid_blocks(allowed_u8, self.cfg.grid_edge).any(axis=(2, 3))
        active = self.active_all & allowed_cell
        if not active.any():
            return []
        return sr.cluster_ink_streams(active)

    def _region_grid_path(self, allowed: np.ndarray) -> list[tuple[int, int]]:
        return sr.flatten_streams(self._region_grid_streams(allowed))

    def _region_skeleton_strokes(self, allowed: np.ndarray) -> list[list[tuple[int, int]]]:
        """骨架模式：区域内墨迹细化 + 8 邻接追踪 + 重采样平滑。"""
        cfg = self.cfg
        region_ink = self.ink_pixels & allowed
        if not region_ink.any():
            return []
        skel = sr._zhang_suen_skeleton(region_ink, max_iterations=160)
        raw = sr.trace_8connected(skel, min_points=cfg.skeleton_min_points)
        if not raw:
            return []
        spacing = cfg.skeleton_resample_spacing
        out: list[list[tuple[int, int]]] = []
        for stroke in raw:
            pts = [(float(x), float(y)) for x, y in stroke]
            pts = sr._resample_stroke_points(pts, spacing)
            pts = sr._chaikin_smooth(pts, iterations=1)
            pts = sr._resample_stroke_points(pts, spacing)
            if len(pts) >= 2 and sr._stroke_cumulative_length(pts)[-1] > 2.0:
                out.append([(int(round(x)), int(round(y))) for x, y in pts])
        return sr._order_skeleton_strokes(out)

    @staticmethod
    def _stroke_bbox(stroke: list[tuple[int, int]]) -> tuple[int, int, int, int]:
        xs = [p[0] for p in stroke]
        ys = [p[1] for p in stroke]
        return min(xs), min(ys), max(xs), max(ys)

    def _group_nearby_strokes(
        self, strokes: list[list[tuple[int, int]]], gap: int = 56
    ) -> list[list[list[tuple[int, int]]]]:
        """近邻笔画成组：表格线/标题框先整组画完，再抬笔去远处的灯泡。"""
        if not strokes:
            return []
        groups: list[list[list[tuple[int, int]]]] = []
        boxes: list[tuple[int, int, int, int]] = []
        for stroke in strokes:
            bb = self._stroke_bbox(stroke)
            hit = [
                i for i, b in enumerate(boxes)
                if not (bb[2] + gap < b[0] or b[2] + gap < bb[0]
                        or bb[3] + gap < b[1] or b[3] + gap < bb[1])
            ]
            if not hit:
                groups.append([stroke])
                boxes.append(bb)
                continue
            target = hit[0]
            groups[target].append(stroke)
            x0, y0, x1, y1 = boxes[target]
            boxes[target] = (min(x0, bb[0]), min(y0, bb[1]), max(x1, bb[2]), max(y1, bb[3]))
            for i in reversed(hit[1:]):
                groups[target].extend(groups.pop(i))
                ob = boxes.pop(i)
                x0, y0, x1, y1 = boxes[target]
                boxes[target] = (min(x0, ob[0]), min(y0, ob[1]), max(x1, ob[2]), max(y1, ob[3]))
        # 组间仍按阅读序：上→下、左→右
        order = sorted(
            range(len(groups)),
            key=lambda i: (boxes[i][1] // 24, boxes[i][0]),
        )
        return [groups[i] for i in order]

    def _finish_stroke_group_ink(
        self, strokes: list[list[tuple[int, int]]], allowed: np.ndarray, pad: int = 18
    ) -> None:
        """一组画完后：把组内笔画附近漏掉的粉笔厚度补齐，再允许抬笔。"""
        if not strokes:
            return
        band = np.zeros((self.out_h, self.out_w), dtype=np.uint8)
        thick = max(3, pad)
        for stroke in strokes:
            if len(stroke) == 1:
                cv2.circle(band, stroke[0], thick, 255, -1)
                continue
            for a, b in zip(stroke, stroke[1:]):
                cv2.line(band, a, b, 255, thickness=thick, lineType=cv2.LINE_AA)
        local = (band > 0) & allowed & self.ink_pixels
        if not local.any():
            return
        self.drawn[local] = self.ink_paint[local]

    @staticmethod
    def _allocate_frames(total: int, weights: list[int]) -> list[int]:
        n = len(weights)
        if total <= 0 or n == 0:
            return [0] * n
        if total < n:
            out = [0] * n
            for i in range(total):
                out[i] = 1
            return out
        tw = sum(max(1, w) for w in weights) or 1
        frames = [max(1, int(total * max(1, w) / tw)) for w in weights]
        # 把差额补到最重的一组，避免合计偏离
        frames[int(max(range(n), key=lambda i: weights[i]))] += total - sum(frames)
        if frames[int(max(range(n), key=lambda i: weights[i]))] < 1:
            frames[int(max(range(n), key=lambda i: weights[i]))] = 1
        return frames

    # ── 落墨（限制在 allowed 内）──
    def _reveal_ink_segment(self, a: tuple[int, int], b: tuple[int, int], allowed: np.ndarray) -> None:
        seg = np.zeros((self.out_h, self.out_w), dtype=np.uint8)
        radius = sr.reveal_radius(self.ink_halfwidth, a, b, self.cfg)
        cv2.line(seg, a, b, 255, thickness=radius * 2 + 1, lineType=cv2.LINE_AA)
        revealed = (seg > 0) & self.ink_pixels & allowed
        self.drawn[revealed] = self.ink_paint[revealed]

    def _ink_stamp_cell(self, cell: tuple[int, int], allowed: np.ndarray) -> None:
        r, c = cell
        e = self.cfg.grid_edge
        block = self.grid_blocks[r, c]
        allow_block = allowed[r * e:r * e + e, c * e:c * e + e]
        ink_region = (block < self.cfg.ink_threshold) & allow_block
        paint = self.ink_paint[r * e:r * e + e, c * e:c * e + e]
        target = self.drawn[r * e:r * e + e, c * e:c * e + e]
        target[ink_region] = paint[ink_region]

    def _color_stamp(self, px: int, py: int, disk: np.ndarray, allowed: np.ndarray) -> None:
        radius = self.cfg.brush_radius
        h, w = self.out_h, self.out_w
        y0, y1 = max(0, py - radius), min(h, py + radius + 1)
        x0, x1 = max(0, px - radius), min(w, px + radius + 1)
        if y1 <= y0 or x1 <= x0:
            return
        by0, by1 = y0 - (py - radius), disk.shape[0] - ((py + radius + 1) - y1)
        bx0, bx1 = x0 - (px - radius), disk.shape[1] - ((px + radius + 1) - x1)
        m = disk[by0:by1, bx0:bx1] * allowed[y0:y1, x0:x1]
        inv = 1.0 - m
        target = self.drawn[y0:y1, x0:x1]
        source = self.color_img[y0:y1, x0:x1].astype(np.float32)
        for ch in range(3):
            target[:, :, ch] = target[:, :, ch] * inv + source[:, :, ch] * m

    # ── 起笔段（骨架模式）：沿笔迹逐段揭原图墨迹，无块填充 ──
    def _hold_frames(self, writer, frames: int) -> None:
        if frames <= 0:
            return
        snap = self.drawn.astype(np.uint8)
        for _ in range(frames):
            self._write_frame(writer, snap)

    def _lay_ink(self, writer, frames: int, samples: list[tuple[int, int]],
                 pen_lifts: set[int], allowed: np.ndarray) -> None:
        if frames <= 0:
            return
        n = len(samples)
        if n == 0:
            self._hold_frames(writer, frames)
            return
        hand_frames = min(frames, max(12, n * 3))
        idx_for_frame = _frame_progress_indices(n, hand_frames)
        last: int | None = None
        for si in idx_for_frame:
            if last is None:
                self._reveal_ink_segment(samples[si], samples[si], allowed)
            else:
                for k in range(last + 1, si + 1):
                    if k in pen_lifts:
                        continue
                    self._reveal_ink_segment(samples[k - 1], samples[k], allowed)
            sx, sy = samples[si]
            self._write_frame(writer, self._snapshot_with_tip(sx, sy))
            last = si
        self._hold_frames(writer, frames - hand_frames)

    # ── 添彩段：brush 或 contour-wipe，限制在 allowed 内 ──
    def _wash_brush(self, writer, frames: int, centers: list[tuple[int, int]], allowed: np.ndarray) -> None:
        if frames <= 0:
            return
        n = len(centers)
        if n == 0:
            snap = self.drawn.astype(np.uint8)
            for _ in range(frames):
                self._write_frame(writer, snap)
            return
        disk = sr._feathered_disk(self.cfg.brush_radius)
        idx_for_frame = _frame_progress_indices(n, frames)
        last: int | None = None
        for ci in idx_for_frame:
            if last is None:
                self._color_stamp(*centers[ci], disk, allowed)
            else:
                for k in range(last + 1, ci + 1):
                    self._color_stamp(*centers[k], disk, allowed)
            cx, cy = centers[ci]
            self._write_frame(writer, self._snapshot_with_tip(cx, cy))
            last = ci

    def _wash_contour(self, writer, frames: int, allowed: np.ndarray) -> None:
        if frames <= 0:
            return
        cfg = self.cfg
        ys_all, xs_all = np.where(allowed)
        if ys_all.size == 0:
            return
        top, bottom = int(ys_all.min()), int(ys_all.max())
        left, right = int(xs_all.min()), int(xs_all.max())
        region_h = bottom - top + 1
        region_w = right - left + 1

        # 区域内的阻力场（墨线膨胀 + 模糊 + 逐行向下衰减）
        ink_u8 = ((self.ink_pixels & allowed)[top:bottom + 1, left:right + 1].astype(np.uint8)) * 255
        spread = int(np.clip(min(region_w, region_h) // 32, 3, 17))
        if spread % 2 == 0:
            spread = max(3, spread - 1)
        kernel = cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (spread, spread))
        dilated = cv2.dilate(ink_u8, kernel, iterations=1)
        blur_r = max(1, int(round(min(region_w, region_h) / 220.0)))
        if blur_r % 2 == 0:
            blur_r += 1
        resistance = cv2.GaussianBlur(dilated, (blur_r, blur_r), 0).astype(np.float32)
        peak = float(resistance.max())
        resistance = resistance / peak if peak > 1e-6 else np.zeros_like(resistance)
        decay = cfg.wipe_decay
        for row in range(1, region_h):
            resistance[row] = np.maximum(resistance[row], resistance[row - 1] * decay)

        wave = sr._build_wipe_wave(region_w)
        delay_px = int(np.clip(region_h * cfg.wipe_delay_ratio, 12, 52))
        ys = np.arange(region_h, dtype=np.float32)[:, None]
        sweep = region_h + 2 * delay_px
        blocks = max(1, cfg.wipe_blocks)

        allowed_crop = allowed[top:bottom + 1, left:right + 1]
        color_crop = self.color_img[top:bottom + 1, left:right + 1].astype(np.float32)
        drawn_crop = self.drawn[top:bottom + 1, left:right + 1]

        for fi in range(frames):
            progress = 1.0 if frames == 1 else fi / (frames - 1)
            lead = sr._ease_in_out_sine(progress) * sweep - delay_px
            threshold = lead + wave[None, :] - resistance * delay_px
            reveal = (ys <= threshold) & allowed_crop
            drawn_crop[reveal] = color_crop[reveal]

            lane = sr._ease_in_out_sine((fi / blocks * 2.0) % 1.0)
            forward = (int(fi // blocks) % 2 == 0)
            cx = int(lane * region_w) if forward else int((1.0 - lane) * region_w)
            cx = max(0, min(region_w - 1, cx))
            col = np.where(reveal[:, cx])[0]
            cy = int(col[-1]) if col.size > 0 else 0
            self._write_frame(writer, self._snapshot_with_tip(left + cx, top + cy))

        # 收尾：确保区域内允许像素全部揭示
        drawn_crop[allowed_crop] = color_crop[allowed_crop]

    # ── 网格路径的采样计划（插值 + 抬笔 + 块填充索引）──
    def _grid_plan(self, path: list[tuple[int, int]]):
        samples: list[tuple[int, int]] = []
        pen_lifts: set[int] = set()
        sample_cell: list[int] = []
        for idx, cell in enumerate(path):
            cx, cy = self._cell_center(cell)
            if idx == 0:
                samples.append((cx, cy))
                sample_cell.append(idx)
                continue
            prev_cell = path[idx - 1]
            prev = self._cell_center(prev_cell)
            if math.hypot(cell[0] - prev_cell[0], cell[1] - prev_cell[1]) > math.sqrt(2):
                pen_lifts.add(len(samples))
                samples.append((cx, cy))
                sample_cell.append(idx)
                continue
            steps = max(1, int(math.hypot(cx - prev[0], cy - prev[1]) / self.cfg.sample_step))
            for s in range(1, steps + 1):
                samples.append((int(prev[0] + (cx - prev[0]) * s / steps),
                                int(prev[1] + (cy - prev[1]) * s / steps)))
                sample_cell.append(idx)
        return samples, pen_lifts, sample_cell

    # ── 主渲染 ──
    def render_to(
        self,
        raw_path: Path,
        total_ms: int,
        *,
        draw_ahead_ms: int | None = None,
        lock_reveal: bool = False,
    ) -> Path:
        cfg = self.cfg
        raw_els = sorted(self.ann.get("elements") or [], key=lambda e: int((e.get("reveal") or {}).get("startMs") or 0))
        ahead = DRAW_AHEAD_MS if draw_ahead_ms is None else max(0, int(draw_ahead_ms))
        elements = []
        weights = []
        for el in raw_els:
            sm = self._shape_mask(el)
            pen = sm is not None and bool(sm.any())
            if not pen:
                print(f"  skip 无框选 {el.get('id') or '?'} {el.get('label') or ''}（手放下）")
            item = dict(el)
            item["reveal"] = dict(el.get("reveal") or {})
            item["_skip_draw"] = not pen
            elements.append(item)
            weights.append(max(400, int(item["reveal"].get("durationMs") or 400)))

        if lock_reveal:
            # 尊重 annotation 里已对齐口播的 startMs/durationMs（TTS_timeline）
            draw_ms = 0
            for item in elements:
                if item.get("_skip_draw"):
                    continue
                draw_ms = max(draw_ms, int(item["reveal"].get("startMs") or 0) + int(item["reveal"].get("durationMs") or 0))
            draw_ms = max(1, min(int(total_ms), draw_ms or int(total_ms)))
        else:
            draw_ms = max(1200, int(total_ms) - ahead)

        fourcc = cv2.VideoWriter_fourcc(*"mp4v")
        writer = cv2.VideoWriter(str(raw_path), fourcc, cfg.fps, (self.out_w, self.out_h))
        if not writer.isOpened():
            raise RuntimeError("无法打开视频写入器")

        self.tip_track = []
        self._record_tips = False
        self._last_tip = None

        weight_sum = max(1, cfg.ink_weight + cfg.color_weight)
        cur_ms = 0.0
        ms_per_frame = 1000.0 / cfg.fps

        def fill_static(until_ms: float) -> None:
            nonlocal cur_ms
            n = int(round((until_ms - cur_ms) / ms_per_frame))
            if n <= 0:
                return
            snap = self.drawn.astype(np.uint8)
            for _ in range(n):
                self._write_frame(writer, snap)
            cur_ms += n * ms_per_frame

        try:
            allowed_list = []
            for i, el in enumerate(elements):
                if el.get("_skip_draw"):
                    allowed_list.append(np.zeros((self.out_h, self.out_w), dtype=bool))
                    continue
                later = [x for x in elements[i + 1 :] if not x.get("_skip_draw")]
                allowed_list.append(self._allowed_mask(el, later))
            covered = np.zeros((self.out_h, self.out_w), dtype=bool)
            for a in allowed_list:
                covered |= a
            unclaimed = self.ink_pixels & ~covered
            drawable_idx = [i for i, el in enumerate(elements) if not el.get("_skip_draw")]
            if unclaimed.any() and drawable_idx:
                centers = []
                for i in drawable_idx:
                    sm = self._shape_mask(elements[i])
                    if sm is not None and sm.any():
                        ys_m, xs_m = np.where(sm)
                        centers.append((float(xs_m.mean()), float(ys_m.mean())))
                    else:
                        x0, y0, x1, y1 = _scaled_rect(elements[i]["region"], self.sx, self.sy, self.out_w, self.out_h)
                        centers.append(((x0 + x1) * 0.5, (y0 + y1) * 0.5))
                ys, xs = np.where(unclaimed)
                d2 = np.stack([
                    (xs.astype(np.float64) - cx) ** 2 + (ys.astype(np.float64) - cy) ** 2
                    for cx, cy in centers
                ], axis=0)
                best = np.argmin(d2, axis=0)
                for j, i in enumerate(drawable_idx):
                    sel = best == j
                    allowed_list[i][ys[sel], xs[sel]] = True
            for i, el in enumerate(elements):
                if el.get("_skip_draw"):
                    continue
                ink_n = int((self.ink_pixels & allowed_list[i]).sum())
                if ink_n < MIN_INK_PX:
                    print(f"  skip 重叠残渣 {el.get('id') or '?'} {el.get('label') or ''}（{ink_n}px，手放下）")
                    el["_skip_draw"] = True
            n_shape = sum(1 for el in elements if not el.get("_skip_draw"))
            if n_shape:
                print(f"  钢笔掩码: {n_shape}/{len(elements)}（不规则形状，无矩形互扣）")
            if not lock_reveal:
                tw = sum(w for w, el in zip(weights, elements) if not el.get("_skip_draw")) or 1
                drawers_left = sum(1 for el in elements if not el.get("_skip_draw"))
                t = 0
                for i, item in enumerate(elements):
                    if item.get("_skip_draw"):
                        item["reveal"]["startMs"] = t
                        item["reveal"]["durationMs"] = 0
                        continue
                    drawers_left -= 1
                    ms = max(1, draw_ms - t) if drawers_left == 0 else max(1, int(round(draw_ms * weights[i] / tw)))
                    item["reveal"]["startMs"] = t
                    item["reveal"]["durationMs"] = ms
                    t += ms
                print(f"  书写 {t}ms / 口播 {total_ms}ms（提前 {max(0, int(total_ms) - t)}ms 画完）")
            else:
                t = sum(int(el["reveal"].get("durationMs") or 0) for el in elements if not el.get("_skip_draw"))
                print(f"  书写(lock) {t}ms / 片长 {total_ms}ms（字级时间轴对齐）")

            for idx, element in enumerate(elements):
                reveal = element["reveal"]
                start_ms = reveal["startMs"]
                dur_ms = reveal["durationMs"]
                fill_static(start_ms)

                allowed = allowed_list[idx]
                if element.get("_skip_draw") or not (self.ink_pixels & allowed).any():
                    fill_static(start_ms + dur_ms)
                    continue
                if cfg.color_weight <= 0:
                    ink_frames = max(1, round(dur_ms * cfg.fps / 1000))
                    color_frames = 0
                else:
                    ink_frames = max(1, round(dur_ms * cfg.ink_weight / weight_sum * cfg.fps / 1000))
                    color_frames = max(1, round(dur_ms * cfg.color_weight / weight_sum * cfg.fps / 1000))

                if cfg.ink_path_mode == "skeleton":
                    strokes = self._region_skeleton_strokes(allowed)
                    if strokes:
                        groups = self._group_nearby_strokes(strokes)
                        weights = [sum(len(s) for s in g) or 1 for g in groups]
                        frame_parts = self._allocate_frames(ink_frames, weights)
                        centers = []
                        for group, gframes in zip(groups, frame_parts):
                            samples: list[tuple[int, int]] = []
                            pen_lifts: set[int] = set()
                            for si, stroke in enumerate(group):
                                if si > 0:
                                    pen_lifts.add(len(samples))
                                samples.extend(stroke)
                            self._lay_ink(writer, gframes, samples, pen_lifts, allowed)
                            self._finish_stroke_group_ink(group, allowed)
                            centers.extend(samples)
                    else:
                        streams = self._region_grid_streams(allowed)
                        if not streams:
                            self._hold_frames(writer, ink_frames)
                            centers = []
                        else:
                            weights = [max(1, len(s)) for s in streams]
                            frame_parts = self._allocate_frames(ink_frames, weights)
                            centers = []
                            for stream, gframes in zip(streams, frame_parts):
                                samples, pen_lifts, sample_cell = self._grid_plan(stream)
                                self._lay_ink_grid(
                                    writer, gframes, samples, pen_lifts, sample_cell, stream, allowed
                                )
                                centers.extend(self._cell_center(c) for c in stream)
                else:
                    streams = self._region_grid_streams(allowed)
                    if streams:
                        weights = [max(1, len(s)) for s in streams]
                        frame_parts = self._allocate_frames(ink_frames, weights)
                        centers = []
                        for stream, gframes in zip(streams, frame_parts):
                            samples, pen_lifts, sample_cell = self._grid_plan(stream)
                            self._lay_ink_grid(
                                writer, gframes, samples, pen_lifts, sample_cell, stream, allowed
                            )
                            centers.extend(self._cell_center(c) for c in stream)
                    else:
                        self._lay_ink(writer, ink_frames, [], set(), allowed)
                        centers = []

                # 收笔：本区允许掩码内漏墨一次补齐（骨架揭墨难免漏像素）
                region_ink = self.ink_pixels & allowed
                self.drawn[region_ink] = self.ink_paint[region_ink]

                cur_ms += ink_frames * ms_per_frame

                if cfg.color_fill == "contour-wipe":
                    self._wash_contour(writer, color_frames, allowed)
                else:
                    self._wash_brush(writer, color_frames, centers, allowed)
                cur_ms += color_frames * ms_per_frame

            # 全局收笔：仍漏的墨迹自上而下补齐，再凝视（不贴整张 color_img）
            remain = self.ink_pixels & (
                np.max(np.abs(self.drawn - self.ink_paint), axis=2) > 6.0
            )
            if remain.any() and cfg.color_weight <= 0:
                ys, xs = np.where(remain)
                order = np.argsort(ys * self.out_w + xs)
                n_fill = min(18, max(6, int(0.35 * cfg.fps)))
                chunk = max(1, len(order) // n_fill)
                for fi in range(n_fill):
                    sel = order[fi * chunk : None if fi == n_fill - 1 else (fi + 1) * chunk]
                    self.drawn[ys[sel], xs[sel]] = self.ink_paint[ys[sel], xs[sel]]
                    self._write_frame(writer, self.drawn.astype(np.uint8))
                    cur_ms += ms_per_frame
            elif cfg.color_weight > 0:
                self.drawn[...] = self.color_img.astype(np.float32)

            fill_static(total_ms)
        finally:
            writer.release()
        return raw_path

    # 网格起笔专用：带块填充，笔尖与揭墨同步
    def _lay_ink_grid(self, writer, frames: int, samples, pen_lifts, sample_cell, path, allowed) -> None:
        if frames <= 0:
            return
        n = len(samples)
        if n == 0:
            self._hold_frames(writer, frames)
            return
        hand_frames = min(frames, max(12, n * 3))
        idx_for_frame = _frame_progress_indices(n, hand_frames)
        cells_done = 0
        last: int | None = None
        for si in idx_for_frame:
            if last is None:
                self._reveal_ink_segment(samples[si], samples[si], allowed)
            else:
                for k in range(last + 1, si + 1):
                    if k in pen_lifts:
                        continue
                    self._reveal_ink_segment(samples[k - 1], samples[k], allowed)
            target_cell = sample_cell[si]
            while cells_done <= target_cell and cells_done < len(path):
                self._ink_stamp_cell(path[cells_done], allowed)
                cells_done += 1
            sx, sy = samples[si]
            self._write_frame(writer, self._snapshot_with_tip(sx, sy))
            last = si
        while cells_done < len(path):
            self._ink_stamp_cell(path[cells_done], allowed)
            cells_done += 1
        self._hold_frames(writer, frames - hand_frames)


def _parse_args(argv=None):
    p = argparse.ArgumentParser(description="SRT 白板动画整合渲染器（mask 编排 + stream 画法）")
    p.add_argument("image", help="线稿图路径")
    p.add_argument("annotation", help="同名 annotation.json 路径")
    p.add_argument("output", help="输出 MP4 路径")
    p.add_argument("hand", nargs="?", default=str(DEFAULT_HAND), help="手部素材 PNG（默认内置）")
    p.add_argument("--total-ms", type=int, default=None, help="总时长；缺省用标注 sceneDurationMs")
    p.add_argument("--bare-tip", action="store_true", help="不叠加笔尖/手部")
    p.add_argument("--ink-path", default="skeleton", choices=["grid", "skeleton"],
                   help="笔迹路径: skeleton 骨架追踪(默认); grid 网格")
    p.add_argument("--color-fill", default="contour-wipe", choices=["contour-wipe", "brush"],
                   help="上色: contour-wipe 轮廓扫描(默认); brush 沿轨迹刷")
    p.add_argument("--pause", default="heavy", choices=["heavy", "auto", "light", "off"],
                   help="起笔段停顿节奏（预留，逐区域画法下影响较弱）")
    p.add_argument("--board-style", default="chalkboard", choices=["chalkboard", "paper"],
                   help="视觉风格: chalkboard 黑砂黑板底(默认); paper 原版暖米黄纸")
    p.add_argument("--fps", type=int, default=None)
    p.add_argument("--grid-edge", type=int, default=None)
    p.add_argument("--ink-weight", type=int, default=None,
                   help="起笔段权重 (默认 2，即 ink:color=2:1)")
    p.add_argument("--color-weight", type=int, default=None,
                   help="添彩段权重；设为 0 则几乎跳过添彩")
    p.add_argument("--brush-radius", type=int, default=None)
    p.add_argument("--cap-long-edge", type=int, default=None,
                   help="输出长边像素上限（默认 1536，与生图对齐）")
    return p.parse_args(argv)


def _build_cfg(args) -> sr.Config:
    kw: dict = {}
    if args.fps is not None:
        kw["fps"] = args.fps
    if args.grid_edge is not None:
        kw["grid_edge"] = args.grid_edge
    if args.ink_weight is not None:
        kw["ink_weight"] = args.ink_weight
    if args.color_weight is not None:
        kw["color_weight"] = args.color_weight
    if args.brush_radius is not None:
        kw["brush_radius"] = args.brush_radius
    if args.cap_long_edge is not None:
        kw["cap_long_edge"] = args.cap_long_edge
    kw["ink_path_mode"] = args.ink_path
    kw["color_fill"] = args.color_fill
    kw["pause_mode"] = args.pause
    kw["board_style"] = args.board_style
    if args.board_style == "paper":
        kw["canvas_hex"] = "#F6F1E3"
        kw["ink_threshold"] = 10
    else:
        kw["canvas_hex"] = "#0b241c"
        kw["ink_threshold"] = 52
    return sr.Config(**kw)


def main(argv=None) -> int:
    args = _parse_args(argv)
    cfg = _build_cfg(args)

    print("=" * 56)
    print("SRT 白板动画整合渲染器 (mask 编排 + stream 画法)")
    print("=" * 56)

    image_bgr = sr._imread_any(args.image)
    if image_bgr is None:
        print(f"[err] 无法读取图片: {args.image}")
        return 1
    try:
        annotation = json.loads(Path(args.annotation).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as e:
        print(f"[err] 无法读取标注: {e}")
        return 1
    if not annotation.get("elements"):
        print("[err] 标注中没有 elements")
        return 1

    total_ms = args.total_ms if args.total_ms is not None else annotation.get("sceneDurationMs")
    if not total_ms:
        last = max(e["reveal"]["startMs"] + e["reveal"]["durationMs"] for e in annotation["elements"])
        total_ms = last + 1000

    out_path = Path(args.output)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    raw_path = out_path.with_name(out_path.stem + "_raw.mp4")

    hand_png = Path(args.hand) if args.hand else None
    ann_path = Path(args.annotation)
    renderer = RegionStreamRenderer(
        image_bgr, annotation, cfg, hand_png, args.bare_tip, ann_dir=ann_path.parent
    )
    print(f"  输入: {args.image}")
    print(f"  标注: {ann_path}")
    print(f"  输出尺寸: {renderer.out_w}x{renderer.out_h}, 帧率: {cfg.fps}")
    print(f"  区域数: {len(annotation['elements'])}, 总时长: {total_ms}ms, "
          f"笔迹: {cfg.ink_path_mode}, 上色: {cfg.color_fill}")

    lock = str(annotation.get("timingSource") or "") == "tts_timeline"
    if lock:
        renderer.render_to(raw_path, int(total_ms), draw_ahead_ms=0, lock_reveal=True)
    else:
        renderer.render_to(raw_path, total_ms)
    final = sr.transcode_h264(raw_path, out_path)

    size_mb = final.stat().st_size / (1024 * 1024)
    print(f"\n最终视频: {final}  ({size_mb:.2f} MB)")
    print("=" * 56)
    print(f"OUTPUT={final}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
