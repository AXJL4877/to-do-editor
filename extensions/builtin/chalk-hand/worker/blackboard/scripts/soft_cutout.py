"""黑底单元素抠图（仅此一步，不做定位/叠层/手部）。

验收标准：
- 纯黑底板透明，不留黑面板晕
- 不啃粉笔：暗部结构、粉尘、字、完整物件都在
- 预览叠在「纯黑+轻微粉笔痕迹」底上给人眼看
"""
from __future__ import annotations

from pathlib import Path

import cv2
import numpy as np

ROOT = Path(__file__).resolve().parent
OUT = ROOT / "cutout"
NAMES = ("bank", "gold", "devalue", "dollar")


def chalk_board_cutout_bgr(raw_bgr: np.ndarray, *, margin: float = 14.0) -> np.ndarray:
    """黑板砂底抠粉笔：放宽白字保留；只清真板面，不啃标签框内字。"""
    h, w = raw_bgr.shape[:2]
    luma = cv2.cvtColor(raw_bgr, cv2.COLOR_BGR2GRAY).astype(np.float32)
    mx = raw_bgr.max(axis=2).astype(np.float32)
    hsv = cv2.cvtColor(raw_bgr, cv2.COLOR_BGR2HSV)
    sat = hsv[:, :, 1].astype(np.float32)

    band = max(8, min(h, w) // 40)
    border = np.concatenate(
        [
            luma[:band, :].ravel(),
            luma[-band:, :].ravel(),
            luma[:, :band].ravel(),
            luma[:, -band:].ravel(),
        ]
    )
    board = float(np.median(border))
    k3 = cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (3, 3))
    k5 = cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (5, 5))

    # 彩粉 / 白粉笔：宁多留，勿抠掉框内字
    color_chalk = (sat > 28.0) & (mx > board + 6.0) & (luma > board + 3.0)
    white_chalk = (sat <= 42.0) & (luma > board + 16.0)
    chalk = color_chalk | white_chalk
    # 彩框邻域内的略亮像素也当墨（黄/红标签里的白字）
    near_color = cv2.dilate(color_chalk.astype(np.uint8) * 255, k5, iterations=10) > 0
    chalk = chalk | (near_color & (luma > board + 8.0) & (mx > board + 6.0))

    ink = chalk.astype(np.uint8) * 255
    ink = cv2.morphologyEx(ink, cv2.MORPH_CLOSE, k3, iterations=3)

    n, lab, st, _ = cv2.connectedComponentsWithStats(ink, 8)
    min_area = max(int(h * w * 0.00003), 20)
    keep_cc = np.zeros(n, dtype=bool)
    for i in range(1, n):
        if int(st[i, cv2.CC_STAT_AREA]) >= min_area:
            keep_cc[i] = True
    ink = (keep_cc[lab]).astype(np.uint8) * 255
    chalk = ink > 0

    dil = cv2.dilate(ink, k5, iterations=3)
    near = dil > 0
    core = (luma > board + 8.0) | ((sat > 20.0) & (mx > board + 5.0))
    holes = near & (ink == 0) & core
    n_h, lab_h, st_h, _ = cv2.connectedComponentsWithStats(holes.astype(np.uint8), 8)
    max_hole = int(h * w * 0.025)
    keep_h = np.zeros(n_h, dtype=bool)
    for i in range(1, n_h):
        a = int(st_h[i, cv2.CC_STAT_AREA])
        if 3 <= a <= max_hole:
            keep_h[i] = True
    holes = keep_h[lab_h]

    strength = np.maximum(0.0, np.maximum(luma, mx) - board)
    soft = np.zeros((h, w), np.float32)
    soft = np.where(chalk, np.clip(0.35 + strength / max(margin * 2.0, 1.0), 0.35, 1.0), soft)
    soft = np.where(holes, np.maximum(soft, np.clip(0.4 + strength / 110.0, 0.4, 0.95)), soft)
    keep = chalk | holes

    ring = cv2.dilate(keep.astype(np.uint8) * 255, k3, iterations=2)
    ring = cv2.subtract(ring, keep.astype(np.uint8) * 255)
    dust = np.clip((strength - 2.0) / 48.0, 0.0, 0.38)
    dust[luma < board + 6.0] = 0.0
    soft = np.where(ring > 0, np.maximum(soft, dust), soft)
    soft = cv2.GaussianBlur(soft.astype(np.float32), (0, 0), 0.5)
    soft = np.where(keep | (ring > 0), soft, 0.0)
    soft[(luma <= board + 6.0) & (sat < 20.0) & (~chalk)] = 0.0

    rgba = cv2.cvtColor(raw_bgr, cv2.COLOR_BGR2BGRA)
    rgba[:, :, 3] = (np.clip(soft, 0, 1) * 255).astype(np.uint8)

    a = rgba[:, :, 3]
    hard = (a >= 24).astype(np.uint8)
    n, lab, st, _ = cv2.connectedComponentsWithStats(hard, 8)
    if n > 1:
        main_id = 1 + int(np.argmax([st[i, cv2.CC_STAT_AREA] for i in range(1, n)]))
        hull = (lab == main_id).astype(np.uint8)
        for i in range(1, n):
            if i == main_id:
                continue
            ar = int(st[i, cv2.CC_STAT_AREA])
            if ar >= max(int(st[main_id, cv2.CC_STAT_AREA] * 0.015), 60):
                hull = cv2.bitwise_or(hull, (lab == i).astype(np.uint8))
        hull = cv2.dilate(hull, k5, iterations=20)
        rgba[:, :, 3] = np.where(hull > 0, a, 0).astype(np.uint8)
    return rgba


def soft_cutout_bgr(raw_bgr: np.ndarray) -> np.ndarray:
    """黑底粉笔抠图：只保留有墨像素；纯黑全透；轻扩一层粉尘羽化。

    暗部结构（发、衣填充、细字）若落在亮墨围成的封闭区域内必须保留。
    不做「轮廓填实」——人群缝、描线空隙必须透，叠到纯黑粉笔底上才像粉笔。
    """
    h, w = raw_bgr.shape[:2]
    luma = cv2.cvtColor(raw_bgr, cv2.COLOR_BGR2GRAY).astype(np.float32)
    # 砂感黑板（边框偏亮）→ 走专用抠板
    band = max(8, min(h, w) // 40)
    border_med = float(
        np.median(
            np.concatenate(
                [
                    luma[:band, :].ravel(),
                    luma[-band:, :].ravel(),
                    luma[:, :band].ravel(),
                    luma[:, -band:].ravel(),
                ]
            )
        )
    )
    if border_med >= 12.0:
        return chalk_board_cutout_bgr(raw_bgr)
    mx = raw_bgr.max(axis=2).astype(np.float32)
    sat = cv2.cvtColor(raw_bgr, cv2.COLOR_BGR2HSV)[:, :, 1].astype(np.float32)

    # HSV sat 在近黑 RGB 上会假高（色噪），必须先有足够亮度才信 sat
    chroma_ok = (mx > 14) & (luma > 8)
    strength = np.maximum(mx, np.maximum(luma, np.where(chroma_ok, sat * 0.5, 0.0)))
    soft = np.clip((strength - 6.0) / 28.0, 0.0, 1.0)
    soft = np.where(
        chroma_ok & (sat > 28),
        np.maximum(soft, np.clip((sat - 20.0) / 50.0, 0, 1)),
        soft,
    )
    soft = np.where(chroma_ok & (sat > 50), np.maximum(soft, 0.82), soft)

    # 亮墨种子（字/描边/彩粉）；禁纯黑高 sat 假墨
    bright = ((mx > 16) | (luma > 16) | (chroma_ok & (sat > 32))).astype(np.uint8) * 255
    k3 = cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (3, 3))
    k5 = cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (5, 5))
    # 只做小闭运算：连细字断笔，不把整幅糊成一片
    bright_closed = cv2.morphologyEx(bright, cv2.MORPH_CLOSE, k3, iterations=2)

    n, lab, st, _ = cv2.connectedComponentsWithStats(bright_closed, 8)
    min_area = max(int(h * w * 0.00006), 80)
    big = np.zeros(n, dtype=bool)
    for i in range(1, n):
        if st[i, cv2.CC_STAT_AREA] >= min_area:
            big[i] = True
    big_mask = big[lab].astype(np.uint8) * 255
    near_big = cv2.dilate(big_mask, k5, iterations=1)
    keep_cc = np.zeros(n, dtype=bool)
    for i in range(1, n):
        area = int(st[i, cv2.CC_STAT_AREA])
        if area >= min_area:
            keep_cc[i] = True
        elif area >= 12 and float((near_big[lab == i] > 0).mean()) > 0.2:
            keep_cc[i] = True
    ink = (keep_cc[lab]).astype(np.uint8) * 255

    # 封闭暗心：从四角 flood 出「真背景」，剩下被亮墨围住的洞 = 脸/衣填充
    bg = np.zeros((h + 2, w + 2), np.uint8)
    inv = cv2.bitwise_not(ink)
    flood = inv.copy()
    cv2.floodFill(flood, bg, (0, 0), 0)
    cv2.floodFill(flood, bg, (w - 1, 0), 0)
    cv2.floodFill(flood, bg, (0, h - 1), 0)
    cv2.floodFill(flood, bg, (w - 1, h - 1), 0)
    holes = (flood > 0) & (ink == 0)
    # 洞内必须真有粉笔结构；纯黑空心不要做成半透明黑罩（会盖掉下层五官/字）
    holes &= (mx > 10) | (luma > 10) | (sat > 18)
    # 丢掉过大的假洞（稀疏噪点围出的整片空区）
    n_h, lab_h, st_h, _ = cv2.connectedComponentsWithStats(holes.astype(np.uint8), 8)
    max_hole = int(h * w * 0.04)
    keep_h = np.zeros(n_h, dtype=bool)
    for i in range(1, n_h):
        if 8 <= int(st_h[i, cv2.CC_STAT_AREA]) <= max_hole:
            keep_h[i] = True
    holes = keep_h[lab_h]
    hole_a = np.clip(0.65 + strength / 70.0, 0.65, 1.0)
    soft = np.where(ink > 0, np.maximum(soft, 0.6), soft)
    soft = np.where(holes, np.maximum(soft, hole_a), soft)

    # 壳外纯黑杀掉（仅 ink∪holes 为前景）
    keep = (ink > 0) | holes
    soft = np.where(keep, soft, 0.0)

    ring = cv2.dilate(keep.astype(np.uint8) * 255, k3, iterations=2)
    ring = cv2.subtract(ring, keep.astype(np.uint8) * 255)
    dust = np.clip((strength - 4.0) / 40.0, 0.0, 0.4)
    dust[(mx <= 5) & (luma <= 5) & (sat <= 10)] = 0.0
    soft = np.where(ring > 0, np.maximum(soft, dust), soft)
    soft = cv2.GaussianBlur(soft.astype(np.float32), (0, 0), 0.6)
    soft = np.where(keep | (ring > 0), soft, 0.0)
    soft[(~keep) & (mx <= 4) & (sat <= 8)] = 0.0

    rgba = cv2.cvtColor(raw_bgr, cv2.COLOR_BGR2BGRA)
    rgba[:, :, 3] = (np.clip(soft, 0, 1) * 255).astype(np.uint8)
    return rgba


def content_bbox(rgba: np.ndarray, thr: int = 20, pad: int = 12) -> tuple[int, int, int, int]:
    a = rgba[:, :, 3] > thr
    if not a.any():
        a = rgba[:, :, 3] > 8
    if not a.any():
        raise RuntimeError("empty cutout")
    ys, xs = np.where(a)
    h, w = a.shape
    x0 = max(0, int(xs.min()) - pad)
    y0 = max(0, int(ys.min()) - pad)
    x1 = min(w, int(xs.max()) + 1 + pad)
    y1 = min(h, int(ys.max()) + 1 + pad)
    return x0, y0, x1, y1


def board_bg(h: int, w: int) -> np.ndarray:
    """验收底板：纯黑 + 极淡粉笔颗粒痕迹（不是墨绿）。"""
    rng = np.random.default_rng(7)
    board = np.zeros((h, w, 3), np.float32)
    # 稀疏粉尘点
    n = max(80, (h * w) // 28000)
    xs = rng.integers(0, w, size=n)
    ys = rng.integers(0, h, size=n)
    for x, y in zip(xs, ys):
        v = float(rng.uniform(14, 36))
        rr = int(rng.integers(1, 3))
        cv2.circle(board, (int(x), int(y)), rr, (v, v, v), -1, lineType=cv2.LINE_AA)
    # 极淡斜向擦痕
    for _ in range(6):
        x0 = int(rng.integers(0, w))
        y0 = int(rng.integers(0, h))
        x1 = int(np.clip(x0 + rng.integers(-180, 180), 0, w - 1))
        y1 = int(np.clip(y0 + rng.integers(-40, 40), 0, h - 1))
        v = float(rng.uniform(8, 18))
        cv2.line(board, (x0, y0), (x1, y1), (v, v, v), 1, lineType=cv2.LINE_AA)
    board = cv2.GaussianBlur(board, (0, 0), 0.6)
    board += rng.normal(0, 1.2, board.shape)
    return np.clip(board, 0, 255).astype(np.uint8)


def over_board(rgba: np.ndarray) -> np.ndarray:
    h, w = rgba.shape[:2]
    bg = board_bg(h, w)
    a = rgba[:, :, 3].astype(np.float32) / 255.0
    a = a[..., None]
    return np.clip(
        rgba[:, :, :3].astype(np.float32) * a + bg.astype(np.float32) * (1.0 - a),
        0,
        255,
    ).astype(np.uint8)


def side_by_side(raw: np.ndarray, onboard: np.ndarray) -> np.ndarray:
    h = max(raw.shape[0], onboard.shape[0])
    w = raw.shape[1] + 20 + onboard.shape[1]
    panel = np.full((h, w, 3), 12, np.uint8)
    panel[: raw.shape[0], : raw.shape[1]] = raw
    panel[: onboard.shape[0], raw.shape[1] + 20 :] = onboard
    cv2.putText(panel, "raw", (24, 48), cv2.FONT_HERSHEY_SIMPLEX, 1.4, (200, 200, 200), 3)
    cv2.putText(
        panel,
        "cutout@black",
        (raw.shape[1] + 44, 48),
        cv2.FONT_HERSHEY_SIMPLEX,
        1.4,
        (180, 255, 180),
        3,
    )
    return panel


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    crops: list[np.ndarray] = []
    labels: list[str] = []

    for name in NAMES:
        path = ROOT / f"raw_{name}.png"
        raw = cv2.imdecode(np.fromfile(str(path), dtype=np.uint8), cv2.IMREAD_COLOR)
        if raw is None:
            raise SystemExit(f"missing {path}")
        rgba = soft_cutout_bgr(raw)
        cov = float((rgba[:, :, 3] > 20).mean())
        x0, y0, x1, y1 = content_bbox(rgba)
        crop = rgba[y0:y1, x0:x1].copy()
        board_full = over_board(rgba)
        board_crop = over_board(crop)
        compare = side_by_side(raw, board_full)

        cv2.imencode(".png", rgba)[1].tofile(str(OUT / f"{name}_full.png"))
        cv2.imencode(".png", crop)[1].tofile(str(OUT / f"{name}_crop.png"))
        cv2.imencode(".png", board_crop)[1].tofile(str(OUT / f"{name}_onboard.png"))
        cv2.imencode(".png", compare)[1].tofile(str(OUT / f"{name}_compare.png"))

        cell, c0, c1 = 24, 48, 72
        hh, ww = crop.shape[:2]
        chk = np.zeros((hh, ww, 3), np.uint8)
        for yy in range(0, hh, cell):
            for xx in range(0, ww, cell):
                chk[yy : yy + cell, xx : xx + cell] = (
                    c0 if ((xx // cell) + (yy // cell)) % 2 == 0 else c1
                )
        aa = crop[:, :, 3].astype(np.float32) / 255.0
        chk_a = (
            crop[:, :, :3].astype(np.float32) * aa[..., None]
            + chk.astype(np.float32) * (1.0 - aa[..., None])
        ).astype(np.uint8)
        cv2.imencode(".png", chk_a)[1].tofile(str(OUT / f"{name}_checker.png"))

        # 黑底残留：不透明像素里仍接近纯黑的比例（越低越好）
        opa = rgba[:, :, 3] > 40
        blackish = opa & (rgba[:, :, :3].max(axis=2) <= 10)
        fringe = float(blackish.sum()) / max(int(opa.sum()), 1)
        print(
            f"{name}: cov={cov:.3f} black_in_opaque={fringe:.4f} "
            f"crop={crop.shape[1]}x{crop.shape[0]}"
        )

        scale = 540 / max(board_crop.shape[0], 1)
        thumb = cv2.resize(
            board_crop,
            (max(1, int(board_crop.shape[1] * scale)), 540),
            interpolation=cv2.INTER_AREA,
        )
        crops.append(thumb)
        labels.append(name)

    row_h = 540
    row_w = max(c.shape[1] for c in crops)
    sheet = np.full((row_h * 2 + 80, row_w * 2 + 60, 3), 18, np.uint8)
    for i, (thumb, name) in enumerate(zip(crops, labels)):
        r, c = divmod(i, 2)
        y = 40 + r * (row_h + 20)
        x = 20 + c * (row_w + 20)
        sheet[y : y + thumb.shape[0], x : x + thumb.shape[1]] = thumb
        cv2.putText(
            sheet,
            name,
            (x + 8, y - 8),
            cv2.FONT_HERSHEY_SIMPLEX,
            1.0,
            (200, 255, 200),
            2,
        )
    cv2.imencode(".png", sheet)[1].tofile(str(OUT / "sheet_onboard.png"))
    print("wrote", OUT / "sheet_onboard.png")


if __name__ == "__main__":
    main()
