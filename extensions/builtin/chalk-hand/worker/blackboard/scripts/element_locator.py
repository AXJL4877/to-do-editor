"""ElementLocator：元素 crop 对分镜做多尺度结构匹配。

(x,y,s) = argmax Similarity(storyboard, Transform(element, x,y,s))
特征：灰度 + Canny；金字塔粗搜 → 邻域精搜；可选色偏 / ORB；已占墨水排除。
"""
from __future__ import annotations

from dataclasses import asdict, dataclass

import cv2
import numpy as np

# 复用同目录抠图（禁止再写第二套抠图逻辑）
from soft_cutout import soft_cutout_bgr  # noqa: F401


@dataclass
class Placement:
    x: int
    y: int
    width: int
    height: int
    scale: float
    confidence: float
    method: str = "pyramid_structure"

    def to_dict(self) -> dict:
        return asdict(self)


def crop_content(rgba: np.ndarray, pad: int = 10) -> np.ndarray:
    """按 alpha 外包盒裁切；保留框内全部墨迹，不丢字/硬币。"""
    a = rgba[:, :, 3] > 16
    if not a.any():
        a = rgba[:, :, 3] > 6
    if not a.any():
        raise RuntimeError("empty cutout")
    ys, xs = np.where(a)
    h, w = a.shape
    x0 = max(0, int(xs.min()) - pad)
    y0 = max(0, int(ys.min()) - pad)
    x1 = min(w, int(xs.max()) + 1 + pad)
    y1 = min(h, int(ys.max()) + 1 + pad)
    return rgba[y0:y1, x0:x1].copy()


def mark_exclude_ink(exclude: np.ndarray, layer_rgba: np.ndarray, inflate: int = 8) -> np.ndarray:
    out = exclude.copy()
    m = (layer_rgba[:, :, 3] > 40).astype(np.uint8) * 255
    if inflate > 0:
        k = cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (inflate * 2 + 1, inflate * 2 + 1))
        m = cv2.dilate(m, k, iterations=1)
    return cv2.bitwise_or(out, m)


def _structure_map(bgr: np.ndarray, *, color_bias: str | None = None) -> np.ndarray:
    g = cv2.cvtColor(bgr, cv2.COLOR_BGR2GRAY)
    g = cv2.GaussianBlur(g, (0, 0), 1.0)
    edges = cv2.Canny(g, 40, 120)
    feat = g.astype(np.float32) * 0.55 + edges.astype(np.float32) * 0.45
    if color_bias == "green":
        hsv = cv2.cvtColor(bgr, cv2.COLOR_BGR2HSV)
        green = (
            ((hsv[:, :, 0] > 35) & (hsv[:, :, 0] < 95) & (hsv[:, :, 1] > 35) & (hsv[:, :, 2] > 40))
            .astype(np.float32)
            * 255.0
        )
        feat = feat * 0.7 + green * 0.3
    elif color_bias == "yellow":
        hsv = cv2.cvtColor(bgr, cv2.COLOR_BGR2HSV)
        yel = (
            (((hsv[:, :, 0] < 35) | (hsv[:, :, 0] > 160)) & (hsv[:, :, 1] > 50) & (hsv[:, :, 2] > 60))
            .astype(np.float32)
            * 255.0
        )
        feat = feat * 0.7 + yel * 0.3
    elif color_bias == "red":
        hsv = cv2.cvtColor(bgr, cv2.COLOR_BGR2HSV)
        red = (
            (((hsv[:, :, 0] < 12) | (hsv[:, :, 0] > 168)) & (hsv[:, :, 1] > 70) & (hsv[:, :, 2] > 50))
            .astype(np.float32)
            * 255.0
        )
        feat = feat * 0.65 + red * 0.35
    return feat


def _masked_structure(
    rgba: np.ndarray, *, color_bias: str | None = None
) -> tuple[np.ndarray, np.ndarray]:
    bgr = rgba[:, :, :3]
    a = rgba[:, :, 3]
    feat = _structure_map(bgr, color_bias=color_bias)
    m = a.astype(np.float32) / 255.0
    feat = feat * m + 40.0 * (1.0 - m)
    mask = (a > 30).astype(np.uint8) * 255
    return feat, mask


def _ncc_at(
    hay: np.ndarray, needle: np.ndarray, mask: np.ndarray | None
) -> tuple[float, tuple[int, int]]:
    if needle.shape[0] >= hay.shape[0] - 2 or needle.shape[1] >= hay.shape[1] - 2:
        return -1.0, (0, 0)
    if needle.shape[0] < 12 or needle.shape[1] < 12:
        return -1.0, (0, 0)
    try:
        if mask is not None and mask.any():
            res = cv2.matchTemplate(hay, needle, cv2.TM_CCOEFF_NORMED, mask=mask)
        else:
            res = cv2.matchTemplate(hay, needle, cv2.TM_CCOEFF_NORMED)
    except cv2.error:
        res = cv2.matchTemplate(hay, needle, cv2.TM_CCOEFF_NORMED)
    _mn, mv, _ml, loc = cv2.minMaxLoc(res)
    return float(mv), (int(loc[0]), int(loc[1]))


def _orb_verify(story_roi_bgr: np.ndarray, elem_bgr: np.ndarray, elem_a: np.ndarray) -> float:
    try:
        orb = cv2.ORB_create(nfeatures=600)
    except Exception:
        return 0.0
    g1 = cv2.cvtColor(story_roi_bgr, cv2.COLOR_BGR2GRAY)
    g2 = cv2.cvtColor(elem_bgr, cv2.COLOR_BGR2GRAY).copy()
    g2[elem_a < 30] = 0
    k1, d1 = orb.detectAndCompute(g1, None)
    k2, d2 = orb.detectAndCompute(g2, None)
    if d1 is None or d2 is None or len(k1) < 8 or len(k2) < 8:
        return 0.0
    bf = cv2.BFMatcher(cv2.NORM_HAMMING, crossCheck=False)
    matches = bf.knnMatch(d2, d1, k=2)
    good = 0
    for pair in matches:
        if len(pair) < 2:
            continue
        m, n = pair
        if m.distance < 0.75 * n.distance:
            good += 1
    return float(np.clip(good / 40.0, 0.0, 1.0))


def locate_element(
    storyboard_bgr: np.ndarray,
    element_rgba: np.ndarray,
    *,
    scale_min: float = 0.55,
    scale_max: float = 1.15,
    exclude_mask: np.ndarray | None = None,
    search_zone: tuple[float, float, float, float] | None = None,
    color_bias: str | None = None,
) -> Placement:
    H, W = storyboard_bgr.shape[:2]
    feat_e, mask_e = _masked_structure(element_rgba, color_bias=color_bias)
    eh0, ew0 = feat_e.shape[:2]

    if search_zone is not None:
        fx0, fy0, fx1, fy1 = search_zone
        zx0, zy0 = int(fx0 * W), int(fy0 * H)
        zx1, zy1 = int(fx1 * W), int(fy1 * H)
    else:
        zx0, zy0, zx1, zy1 = 0, 0, W, H

    story = storyboard_bgr[zy0:zy1, zx0:zx1]
    feat_s_full = _structure_map(story, color_bias=color_bias)
    sh, sw = feat_s_full.shape[:2]

    if exclude_mask is not None:
        excl = exclude_mask[zy0:zy1, zx0:zx1]
        feat_s_full = feat_s_full.copy()
        feat_s_full[excl > 0] = 40.0

    pyr = 0.28
    scales_coarse = [round(x, 3) for x in np.arange(scale_min, scale_max + 1e-6, 0.05)]
    hay1 = cv2.resize(feat_s_full, None, fx=pyr, fy=pyr, interpolation=cv2.INTER_AREA)
    candidates: list[dict] = []
    for sc in scales_coarse:
        tw = max(14, int(round(ew0 * sc * pyr)))
        th = max(14, int(round(eh0 * sc * pyr)))
        if tw >= hay1.shape[1] - 2 or th >= hay1.shape[0] - 2:
            continue
        needle = cv2.resize(feat_e, (int(ew0 * sc), int(eh0 * sc)), interpolation=cv2.INTER_AREA)
        needle = cv2.resize(needle, (tw, th), interpolation=cv2.INTER_AREA)
        m = cv2.resize(mask_e, (tw, th), interpolation=cv2.INTER_AREA)
        m = (m > 30).astype(np.uint8) * 255
        ncc, (lx, ly) = _ncc_at(hay1, needle, m)
        if ncc < 0:
            continue
        candidates.append(
            {
                "ncc": ncc,
                "scale": float(sc),
                "x_l": int(round(lx / pyr)),
                "y_l": int(round(ly / pyr)),
                "w": max(1, int(round(ew0 * sc))),
                "h": max(1, int(round(eh0 * sc))),
            }
        )

    if not candidates:
        sc = float(np.clip((scale_min + scale_max) / 2, 0.5, 1.2))
        tw, th = max(1, int(ew0 * sc)), max(1, int(eh0 * sc))
        return Placement(
            x=max(0, zx0 + (sw - tw) // 2),
            y=max(0, zy0 + (sh - th) // 2),
            width=tw,
            height=th,
            scale=sc,
            confidence=0.0,
            method="fallback_center",
        )

    candidates.sort(key=lambda c: -c["ncc"])
    top = candidates[:10]
    refined: list[dict] = []
    for seed in top[:5]:
        sc0 = seed["scale"]
        fine_scales = [
            round(sc0 + d, 3)
            for d in np.arange(-0.06, 0.07, 0.02)
            if scale_min - 0.02 <= sc0 + d <= scale_max + 0.02
        ]
        for sc in fine_scales:
            tw = max(1, int(round(ew0 * sc)))
            th = max(1, int(round(eh0 * sc)))
            if tw >= sw - 4 or th >= sh - 4:
                continue
            needle = cv2.resize(feat_e, (tw, th), interpolation=cv2.INTER_AREA)
            m = cv2.resize(mask_e, (tw, th), interpolation=cv2.INTER_AREA)
            m = (m > 30).astype(np.uint8) * 255
            cx = seed["x_l"] + seed["w"] // 2
            cy = seed["y_l"] + seed["h"] // 2
            pad = 100
            x0 = max(0, cx - tw // 2 - pad)
            y0 = max(0, cy - th // 2 - pad)
            x1 = min(sw, cx + tw // 2 + pad)
            y1 = min(sh, cy + th // 2 + pad)
            if x1 - x0 <= tw + 2 or y1 - y0 <= th + 2:
                continue
            hay = feat_s_full[y0:y1, x0:x1]
            step = 0.25
            hay_s = cv2.resize(hay, None, fx=step, fy=step, interpolation=cv2.INTER_AREA)
            nd_s = cv2.resize(needle, None, fx=step, fy=step, interpolation=cv2.INTER_AREA)
            m_s = cv2.resize(m, (nd_s.shape[1], nd_s.shape[0]), interpolation=cv2.INTER_AREA)
            m_s = (m_s > 30).astype(np.uint8) * 255
            ncc, (lx, ly) = _ncc_at(hay_s, nd_s, m_s)
            if ncc < 0:
                continue
            refined.append(
                {
                    "ncc": ncc,
                    "scale": float(sc),
                    "x_l": int(round(x0 + lx / step)),
                    "y_l": int(round(y0 + ly / step)),
                    "w": tw,
                    "h": th,
                }
            )

    pool = refined if refined else top
    pool.sort(key=lambda c: -c["ncc"])
    best = pool[0]

    sc = best["scale"]
    tw, th = best["w"], best["h"]
    needle = cv2.resize(feat_e, (tw, th), interpolation=cv2.INTER_AREA)
    m = cv2.resize(mask_e, (tw, th), interpolation=cv2.INTER_AREA)
    m = (m > 30).astype(np.uint8) * 255
    cx = best["x_l"] + tw // 2
    cy = best["y_l"] + th // 2
    pad = 24
    x0 = max(0, cx - tw // 2 - pad)
    y0 = max(0, cy - th // 2 - pad)
    x1 = min(sw, cx + tw // 2 + pad)
    y1 = min(sh, cy + th // 2 + pad)
    if x1 - x0 > tw + 2 and y1 - y0 > th + 2:
        hay = feat_s_full[y0:y1, x0:x1]
        ncc, (lx, ly) = _ncc_at(hay, needle, m)
        if ncc >= best["ncc"] - 1e-6:
            best = {
                "ncc": ncc,
                "scale": sc,
                "x_l": x0 + lx,
                "y_l": y0 + ly,
                "w": tw,
                "h": th,
            }

    gx = zx0 + best["x_l"]
    gy = zy0 + best["y_l"]
    gw, gh = best["w"], best["h"]
    gx1, gy1 = min(W, gx + gw), min(H, gy + gh)
    roi = storyboard_bgr[gy:gy1, gx:gx1]
    elem_bgr = cv2.resize(element_rgba[:, :, :3], (gw, gh), interpolation=cv2.INTER_AREA)
    elem_a = cv2.resize(element_rgba[:, :, 3], (gw, gh), interpolation=cv2.INTER_AREA)
    orb_s = 0.0
    if roi.size and roi.shape[0] > 20 and roi.shape[1] > 20:
        orb_s = _orb_verify(roi, elem_bgr, elem_a)

    conf = float(np.clip(0.85 * best["ncc"] + 0.15 * orb_s, 0.0, 1.0))
    return Placement(
        x=int(gx),
        y=int(gy),
        width=int(gw),
        height=int(gh),
        scale=float(best["scale"]),
        confidence=conf,
        method="pyramid_structure+orb",
    )


def refine_scale(
    storyboard_bgr: np.ndarray,
    element_rgba: np.ndarray,
    place: Placement,
    *,
    scale_min: float,
    scale_max: float,
    color_bias: str | None = None,
    geo_hint: float | None = None,
) -> Placement:
    """固定中心，密搜 scale；可选 geo_hint（分镜区墨迹尺度）作弱先验。"""
    H, W = storyboard_bgr.shape[:2]
    feat_e, mask_e = _masked_structure(element_rgba, color_bias=color_bias)
    eh0, ew0 = feat_e.shape[:2]
    feat_s = _structure_map(storyboard_bgr, color_bias=color_bias)
    cx = place.x + place.width / 2.0
    cy = place.y + place.height / 2.0

    scales = sorted(
        {
            round(s, 3)
            for s in np.arange(scale_min, scale_max + 1e-6, 0.025)
        }
        | ({round(float(geo_hint), 3)} if geo_hint else set())
        | {round(place.scale, 3)}
    )
    best = None
    for sc in scales:
        if sc < scale_min - 1e-6 or sc > scale_max + 1e-6:
            continue
        tw = max(8, int(round(ew0 * sc)))
        th = max(8, int(round(eh0 * sc)))
        if tw >= W - 4 or th >= H - 4:
            continue
        x = int(round(cx - tw / 2))
        y = int(round(cy - th / 2))
        x = int(np.clip(x, 0, W - tw))
        y = int(np.clip(y, 0, H - th))
        needle = cv2.resize(feat_e, (tw, th), interpolation=cv2.INTER_AREA)
        m = cv2.resize(mask_e, (tw, th), interpolation=cv2.INTER_AREA)
        m = (m > 30).astype(np.uint8) * 255
        hay = feat_s[y : y + th, x : x + tw]
        if hay.shape[:2] != (th, tw):
            continue
        # 局部相关：归一化
        try:
            if m.any():
                res = cv2.matchTemplate(
                    feat_s[max(0, y - 2) : min(H, y + th + 2), max(0, x - 2) : min(W, x + tw + 2)],
                    needle,
                    cv2.TM_CCOEFF_NORMED,
                    mask=m,
                )
                ncc = float(res.max()) if res.size else -1.0
            else:
                ncc = -1.0
        except cv2.error:
            ncc = -1.0
        if ncc < 0:
            continue
        score = ncc
        if geo_hint is not None:
            # 弱拉向分镜墨迹尺度，避免 NCC 锁在错误小尺度
            score = 0.55 * ncc + 0.45 * (1.0 - min(1.0, abs(sc - geo_hint) / max(0.35, geo_hint)))
        if best is None or score > best["score"]:
            best = {
                "score": score,
                "ncc": ncc,
                "scale": float(sc),
                "x": x,
                "y": y,
                "w": tw,
                "h": th,
            }

    if best is None:
        return place
    conf = float(np.clip(0.85 * best["ncc"] + 0.15 * place.confidence, 0.0, 1.0))
    return Placement(
        x=int(best["x"]),
        y=int(best["y"]),
        width=int(best["w"]),
        height=int(best["h"]),
        scale=float(best["scale"]),
        confidence=conf,
        method=place.method + "+scale_refine",
    )


def place_on_canvas(canvas_h: int, canvas_w: int, crop_rgba: np.ndarray, p: Placement) -> np.ndarray:
    tw, th = p.width, p.height
    scaled = cv2.resize(crop_rgba, (tw, th), interpolation=cv2.INTER_LINEAR)
    full = np.zeros((canvas_h, canvas_w, 4), np.uint8)
    x, y = p.x, p.y
    x0, y0 = max(0, x), max(0, y)
    x1, y1 = min(canvas_w, x + tw), min(canvas_h, y + th)
    if x1 > x0 and y1 > y0:
        full[y0:y1, x0:x1] = scaled[y0 - y : y1 - y, x0 - x : x1 - x]
    return full
