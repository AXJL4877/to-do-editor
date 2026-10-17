"""黑底元素管线（标准入口）

分镜图（软参考）→ 各元素黑底重生 → soft_cutout → ElementLocator 叠纯黑粉笔底
→ 每元素独立 bbox mask 渐进手部 → mux/字幕。

用法：
  python scripts/blackbg_element_hand.py out/2026-09-17 02 --stage all
  python scripts/blackbg_element_hand.py out/2026-09-17 02 --stage gen
  python scripts/blackbg_element_hand.py out/2026-09-17 02 --stage cutout,place,hand,mux
"""
from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
import time
from pathlib import Path

import cv2
import numpy as np

REPO = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO / "scripts"))
sys.path.insert(0, str(REPO))

from soft_cutout import soft_cutout_bgr, content_bbox  # noqa: E402
from element_locator import (  # noqa: E402
    Placement,
    crop_content,
    locate_element,
    mark_exclude_ink,
    place_on_canvas,
    refine_scale,
)


def _load_bgr(path: Path) -> np.ndarray:
    img = cv2.imdecode(np.fromfile(str(path), dtype=np.uint8), cv2.IMREAD_COLOR)
    if img is None:
        raise FileNotFoundError(path)
    return img


def _load_rgba(path: Path) -> np.ndarray:
    img = cv2.imdecode(np.fromfile(str(path), dtype=np.uint8), cv2.IMREAD_UNCHANGED)
    if img is None or img.ndim < 3 or img.shape[2] < 4:
        raise FileNotFoundError(path)
    return img


def _save(path: Path, img: np.ndarray) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    cv2.imencode(".png", img)[1].tofile(str(path))


def over(dst: np.ndarray, rgba: np.ndarray) -> np.ndarray:
    bgr = rgba[:, :, :3]
    a = rgba[:, :, 3].astype(np.float32) / 255.0
    mx = bgr.max(axis=2).astype(np.float32)
    a = np.where(mx <= 4.0, 0.0, a)[..., None]
    return np.clip(bgr.astype(np.float32) * a + dst.astype(np.float32) * (1.0 - a), 0, 255).astype(
        np.uint8
    )


def make_board(h: int, w: int, board_path: Path | None) -> np.ndarray:
    if board_path and board_path.exists():
        b = _load_bgr(board_path)
        if b.shape[0] != h or b.shape[1] != w:
            b = cv2.resize(b, (w, h), interpolation=cv2.INTER_AREA)
        return b
    rng = np.random.default_rng(5)
    board = np.zeros((h, w, 3), np.float32)
    n = max(80, (h * w) // 28000)
    for x, y in zip(rng.integers(0, w, n), rng.integers(0, h, n)):
        v = float(rng.uniform(14, 36))
        cv2.circle(board, (int(x), int(y)), int(rng.integers(1, 3)), (v, v, v), -1, lineType=cv2.LINE_AA)
    board = cv2.GaussianBlur(board, (0, 0), 0.6)
    board += rng.normal(0, 1.2, board.shape)
    return np.clip(board, 0, 255).astype(np.uint8)


def audio_ms(path: Path) -> int:
    r = subprocess.run(
        [
            "ffprobe",
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            str(path),
        ],
        capture_output=True,
        text=True,
        check=True,
    )
    return int(round(float(r.stdout.strip()) * 1000))


def split_timings(total_ms: int, n: int, weights: list[float] | None = None) -> list[tuple[int, int]]:
    if n <= 0:
        return []
    w = weights or [1.0] * n
    s = sum(w) or 1.0
    # 书写在口播结束前约 2s 画完（与 DRAW_AHEAD 对齐）
    draw_budget = max(n * 1200, total_ms - 2000)
    out: list[tuple[int, int]] = []
    t = 0
    for i in range(n):
        dur = int(round(draw_budget * w[i] / s)) if i < n - 1 else max(800, draw_budget - t)
        out.append((t, max(800, dur)))
        t += out[-1][1]
    return out


# ── stages ──────────────────────────────────────────────────────────


def stage_gen(work: Path, shot: Path, cfg: dict, timing: dict) -> None:
    """黑底生元素。layout 模式只用风格板，不用分镜图锁位置。"""
    raw_dir = work / "raw"
    raw_dir.mkdir(parents=True, exist_ok=True)
    place_mode = cfg.get("place_mode", "storyboard_strict")
    if place_mode == "layout":
        ref = REPO / cfg.get("style_ref", "assets/style.png")
        host_path = f"blackboard-auto/{shot.parent.name}/{shot.name}-style.png"
    else:
        ref = shot / "image.png"
        host_path = f"blackboard-auto/{shot.parent.name}/{shot.name}-ref.png"
    if not ref.exists():
        raise FileNotFoundError(ref)

    t0 = time.perf_counter()
    host = subprocess.run(
        [
            sys.executable,
            str(Path("run.py")),
            "host",
            "--file",
            str(ref),
            "--path",
            host_path,
        ],
        capture_output=True,
        text=True,
        check=True,
    )
    line = [ln for ln in host.stdout.splitlines() if ln.strip().startswith("{")][-1]
    data = json.loads(line)
    ref_url = data.get("url") or data.get("public_url") or data.get("cdn") or data.get("href")
    if not ref_url:
        raise RuntimeError(f"host failed: {host.stdout[-500:]}")
    timing["host_s"] = round(time.perf_counter() - t0, 2)
    print(f"  host {timing['host_s']}s → {ref_url[:80]}...")

    els = cfg["elements"]
    names = []
    t1 = time.perf_counter()
    for el in els:
        name = el["id"]
        names.append(name)
        prompt = el["prompt"]
        if place_mode == "layout":
            prompt = (
                prompt
                + " Match the chalk-on-blackboard grain/style of the reference; "
                "ignore any composition in the reference — subject must be centered on pure black."
            )
        else:
            prompt = prompt.replace(
                "{REF_HINT}",
                "Use the reference chalkboard for style and approximate layout.",
            )
        out = raw_dir / f"{name}.png"
        cmd = [
            sys.executable,
            str(Path("run.py")),
            "image",
            "--size",
            "3840x2160",
            "--image",
            ref_url,
            "--prompt",
            prompt,
            "--name",
            f"bb_{shot.name}_{name}",
            "--out",
            str(out),
            "--no-wait",
        ]
        print(f"  submit {name}...")
        subprocess.run(cmd, check=True)

    for name in names:
        out = raw_dir / f"{name}.png"
        print(f"  poll {name}...")
        subprocess.run(
            [
                sys.executable,
                str(Path("run.py")),
                "image",
                "--poll",
                f"bb_{shot.name}_{name}",
                "--out",
                str(out),
            ],
            check=True,
        )
        if not out.exists() or out.stat().st_size < 1000:
            raise RuntimeError(f"gen missing: {out}")
    timing["gen_s"] = round(time.perf_counter() - t1, 2)
    print(f"  gen {timing['gen_s']}s ({len(names)} elements parallel)")


def stage_cutout(work: Path, cfg: dict, timing: dict) -> None:
    t0 = time.perf_counter()
    el_dir = work / "elements"
    el_dir.mkdir(parents=True, exist_ok=True)
    meta: dict = {}
    for el in cfg["elements"]:
        name = el["id"]
        raw = _load_bgr(work / "raw" / f"{name}.png")
        rgba = soft_cutout_bgr(raw)
        x0, y0, x1, y1 = content_bbox(rgba)
        crop = rgba[y0:y1, x0:x1].copy()
        _save(el_dir / f"{name}_full.png", rgba)
        _save(el_dir / f"{name}.png", crop)
        meta[name] = {
            "x": x0,
            "y": y0,
            "width": x1 - x0,
            "height": y1 - y0,
            "claimed_x": x0,
            "claimed_y": y0,
            "claimed_w": x1 - x0,
            "claimed_h": y1 - y0,
        }
        cov = float((rgba[:, :, 3] > 20).mean())
        print(f"  cutout {name}: crop={crop.shape[1]}x{crop.shape[0]} cov={cov:.3f}")
    (el_dir / "meta.json").write_text(json.dumps(meta, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    timing["cutout_s"] = round(time.perf_counter() - t0, 2)
    print(f"  cutout {timing['cutout_s']}s")


# 叙事向构图模板：归一化 zone 列表生成器（按口播顺序）。禁止均分网格同质化。
LAYOUT_PRESETS = (
    "hero_left",
    "diagonal_fall",
    "conveyor_wave",
    "fork_right",
    "stack_rise",
    "climax_right",
)


def _nz(zones: list[tuple[float, float, float, float]], n: int) -> list[tuple[float, float, float, float]]:
    """裁剪/补齐到 n 个区；不足时在末尾右侧递推。"""
    if n <= 0:
        return []
    if len(zones) >= n:
        return zones[:n]
    out = list(zones)
    while len(out) < n:
        i = len(out)
        x0 = min(0.82, 0.08 + 0.14 * (i % 5))
        y0 = min(0.70, 0.10 + 0.12 * (i // 2))
        out.append((x0, y0, min(0.99, x0 + 0.28), min(0.95, y0 + 0.35)))
    return out


def expressive_zones(n: int, preset: str) -> list[tuple[float, float, float, float]]:
    """返回 n 个 (x0,y0,x1,y1) 归一化分区，口播顺序 = 叙事推进。"""
    p = preset
    if p == "hero_left":
        # 左巨幅开场，其余沿右上→右下递进
        base = [
            (0.00, 0.06, 0.52, 0.92),
            (0.42, 0.04, 0.78, 0.48),
            (0.70, 0.02, 0.99, 0.36),
            (0.40, 0.48, 0.72, 0.96),
            (0.62, 0.38, 0.96, 0.78),
            (0.74, 0.68, 1.00, 0.99),
        ]
    elif p == "diagonal_fall":
        # 左上→右下斜切，尺度渐变
        base = []
        for i in range(max(n, 6)):
            t = i / max(n - 1, 1)
            w = 0.42 - 0.04 * (i % 3)
            h = 0.48 - 0.03 * (i % 2)
            x0 = 0.02 + t * 0.52
            y0 = 0.04 + t * 0.42
            base.append((x0, y0, min(0.99, x0 + w), min(0.98, y0 + h)))
    elif p == "conveyor_wave":
        # 水平传送带感：高低交错
        ys = [0.28, 0.08, 0.38, 0.12, 0.42, 0.18]
        base = []
        for i in range(max(n, 6)):
            x0 = 0.01 + i * (0.92 / max(n, 1))
            y0 = ys[i % len(ys)]
            base.append((x0, y0, min(0.99, x0 + 0.30), min(0.95, y0 + 0.55)))
    elif p == "fork_right":
        # 左主干，右侧上下分叉收束
        base = [
            (0.00, 0.18, 0.40, 0.88),
            (0.32, 0.02, 0.68, 0.42),
            (0.32, 0.48, 0.68, 0.95),
            (0.60, 0.05, 0.92, 0.45),
            (0.60, 0.48, 0.92, 0.92),
            (0.78, 0.30, 1.00, 0.70),
        ]
    elif p == "stack_rise":
        # 底层大、上层小，由下往上叠（时间向前推高）
        base = [
            (0.02, 0.42, 0.48, 0.98),
            (0.28, 0.28, 0.68, 0.78),
            (0.48, 0.12, 0.88, 0.58),
            (0.05, 0.05, 0.38, 0.40),
            (0.58, 0.48, 0.95, 0.95),
            (0.72, 0.02, 0.99, 0.36),
        ]
    elif p == "climax_right":
        # 前奏散在左缘，高潮占右半巨幅，收束贴右下
        base = [
            (0.00, 0.08, 0.34, 0.55),
            (0.08, 0.48, 0.40, 0.92),
            (0.28, 0.02, 0.55, 0.38),
            (0.30, 0.40, 0.58, 0.78),
            (0.48, 0.12, 0.92, 0.82),
            (0.70, 0.68, 1.00, 0.99),
        ]
    else:
        base = expressive_zones(n, "hero_left")
        return base[:n] if len(base) >= n else _nz(base, n)
    return _nz(base, n)


def pick_layout_preset(shot: Path, cfg: dict, n: int) -> str:
    """显式 layout 优先；auto 则按镜号轮换，避免镜镜同构。"""
    raw = str(cfg.get("layout", "auto")).strip().lower()
    if raw in LAYOUT_PRESETS or raw in ("zones", "grid", "ltr"):
        return raw
    try:
        idx = int(shot.name) - 1
    except ValueError:
        idx = sum(ord(c) for c in shot.name)
    pool = list(LAYOUT_PRESETS)
    if n <= 3:
        pool = ["hero_left", "conveyor_wave", "diagonal_fall"]
    elif n == 4:
        pool = ["fork_right", "hero_left", "climax_right", "diagonal_fall"]
    return pool[idx % len(pool)]


def layout_slot_rects(
    n: int,
    W: int,
    H: int,
    *,
    layout: str = "grid",
    cols: int | None = None,
    margin: float = 0.04,
    gap: float = 0.02,
) -> list[tuple[int, int, int, int]]:
    """按口播顺序均分画幅；grid 默认多列换行，ltr 单行。返回 (x0,y0,x1,y1)。"""
    if n <= 0:
        return []
    if layout == "ltr":
        cols = n
        rows = 1
    else:
        cols = int(cols or (3 if n >= 5 else n))
        cols = max(1, min(cols, n))
        rows = (n + cols - 1) // cols
    mx, my = int(W * margin), int(H * margin)
    gx, gy = int(W * gap), int(H * gap)
    inner_w = W - 2 * mx - gx * (cols - 1)
    inner_h = H - 2 * my - gy * (rows - 1)
    cw, ch = max(8, inner_w // cols), max(8, inner_h // rows)
    rects: list[tuple[int, int, int, int]] = []
    for i in range(n):
        r, c = divmod(i, cols)
        x0 = mx + c * (cw + gx)
        y0 = my + r * (ch + gy)
        rects.append((x0, y0, x0 + cw, y0 + ch))
    return rects


def zones_to_rects(
    zones: list[tuple[float, float, float, float]], W: int, H: int
) -> list[tuple[int, int, int, int]]:
    return [
        (int(z[0] * W), int(z[1] * H), int(z[2] * W), int(z[3] * H))
        for z in zones
    ]


def place_in_rect(
    crop: np.ndarray,
    rect: tuple[int, int, int, int],
    H: int,
    W: int,
    *,
    fill: float = 0.88,
) -> Placement:
    x0, y0, x1, y1 = rect
    zw, zh = max(8, x1 - x0), max(8, y1 - y0)
    ew, eh = int(crop.shape[1]), int(crop.shape[0])
    sc = min(zw / ew, zh / eh) * float(fill)
    tw, th = max(8, int(round(ew * sc))), max(8, int(round(eh * sc)))
    x = int(np.clip(x0 + (zw - tw) // 2, 0, max(0, W - tw)))
    y = int(np.clip(y0 + (zh - th) // 2, 0, max(0, H - th)))
    return Placement(
        x=x, y=y, width=tw, height=th, scale=float(sc), confidence=1.0, method="narration_layout"
    )


def stage_place(work: Path, shot: Path, cfg: dict, timing: dict) -> list[tuple[str, np.ndarray]]:
    """layout：程序槽位；split_identity：拆分 full 原位叠黑板；storyboard_*：对齐分镜。"""
    t0 = time.perf_counter()
    place_mode = cfg.get("place_mode", "storyboard_strict")
    canvas_cfg = cfg.get("canvas") or {}
    story_path = work / "storyboard.png"
    if place_mode == "layout":
        W = int(canvas_cfg.get("width", 3840))
        H = int(canvas_cfg.get("height", 2160))
        src = None
    elif place_mode == "split_identity":
        probe = work / "elements" / f"{cfg['elements'][0]['id']}_full.png"
        if probe.exists():
            sample = _load_rgba(probe)
            H, W = int(sample.shape[0]), int(sample.shape[1])
        elif story_path.exists():
            src0 = _load_bgr(story_path)
            H, W = src0.shape[:2]
        else:
            W = int(canvas_cfg.get("width", 3840))
            H = int(canvas_cfg.get("height", 2160))
        src = _load_bgr(story_path) if story_path.exists() else None
        print(f"  place mode=split_identity {W}x{H}")
    else:
        if story_path.exists():
            src = _load_bgr(story_path)
            print(f"  place storyboard={story_path.name}")
        else:
            src = _load_bgr(shot / "image.png")
            print(f"  place storyboard=shot/image.png (no {story_path.name})")
        H, W = src.shape[:2]

    board_path = Path(cfg.get("board") or "")
    if not board_path.is_absolute():
        board_path = (
            REPO / board_path if board_path else REPO / "out/2026-09-17/_pilot_blackbg/board_black_chalk.png"
        )
    board = make_board(H, W, board_path)
    _save(work / "board.png", board)

    placed: list[tuple[str, np.ndarray]] = []
    canvas = board.copy()
    fill_default = float(cfg.get("zone_fill", 0.88))
    report: dict = {"layers": [], "mode": place_mode}

    if place_mode == "split_identity":
        for el in cfg["elements"]:
            name = el["id"]
            full_path = work / "elements" / f"{name}_full.png"
            if not full_path.exists():
                raise FileNotFoundError(f"split_identity needs {full_path}")
            full = _load_rgba(full_path)
            if full.shape[0] != H or full.shape[1] != W:
                full = cv2.resize(full, (W, H), interpolation=cv2.INTER_AREA)
            _save(work / f"aligned_{name}.png", full)
            placed.append((name, full))
            canvas = over(canvas, full)
            ys, xs = np.where(full[:, :, 3] > 20)
            if ys.size:
                x0, y0 = int(xs.min()), int(ys.min())
                bw, bh = int(xs.max() - xs.min() + 1), int(ys.max() - ys.min() + 1)
            else:
                x0 = y0 = bw = bh = 0
            print(f"  place {name}: identity bbox=({x0},{y0}) {bw}x{bh}")
            report["layers"].append(
                {
                    "id": name,
                    "x": x0,
                    "y": y0,
                    "width": bw,
                    "height": bh,
                    "scale": 1.0,
                    "confidence": 1.0,
                    "method": "split_identity",
                }
            )
    elif place_mode == "layout":
        n = len(cfg["elements"])
        layout_kind = pick_layout_preset(shot, cfg, n)
        if layout_kind == "zones" and all(el.get("zone") for el in cfg["elements"]):
            rects = zones_to_rects(
                [tuple(el["zone"]) for el in cfg["elements"]], W, H  # type: ignore[arg-type]
            )
        elif layout_kind in ("grid", "ltr"):
            rects = layout_slot_rects(n, W, H, layout=layout_kind, cols=cfg.get("layout_cols"))
        else:
            # 表现力模板（含 auto 解析结果）
            if layout_kind == "zones":
                layout_kind = pick_layout_preset(shot, {**cfg, "layout": "auto"}, n)
            rects = zones_to_rects(expressive_zones(n, layout_kind), W, H)
        report["layout_preset"] = layout_kind
        print(f"  layout preset={layout_kind}")

        # 权重越大略放大填充，增强主次
        weights = [float(el.get("weight", 1.0)) for el in cfg["elements"]]
        w_max = max(weights) if weights else 1.0

        for el, rect, wt in zip(cfg["elements"], rects, weights):
            name = el["id"]
            crop = _load_rgba(work / "elements" / f"{name}.png")
            fill = float(el.get("zone_fill", fill_default))
            fill = float(np.clip(fill * (0.92 + 0.10 * (wt / w_max)), 0.82, 0.98))
            place = place_in_rect(crop, rect, H, W, fill=fill)
            print(
                f"  place {name}: ({place.x},{place.y}) {place.width}x{place.height} "
                f"sc={place.scale:.2f} [{place.method}] slot={rect}"
            )
            full = place_on_canvas(H, W, crop, place)
            _save(work / f"aligned_{name}.png", full)
            placed.append((name, full))
            canvas = over(canvas, full)
            report["layers"].append({"id": name, **place.to_dict(), "slot": list(rect)})
    else:
        exclude = np.zeros((H, W), np.uint8)
        g = cv2.cvtColor(src, cv2.COLOR_BGR2GRAY)
        hsv = cv2.cvtColor(src, cv2.COLOR_BGR2HSV)
        ink = ((g > 55) | (hsv[:, :, 1] > 40)).astype(np.uint8) * 255
        # locate 接受门槛：过低会把弱 NCC 误匹配当真（take_gold sc=0.16/conf=0.22）
        loc_conf_min = float(cfg.get("locate_conf_min", 0.45))
        loc_scale_floor = float(cfg.get("locate_scale_floor", 0.35))
        loc_vs_zone_min = float(cfg.get("locate_vs_zone_min", 0.55))

        for el in cfg["elements"]:
            name = el["id"]
            zone = tuple(el["zone"])
            crop = _load_rgba(work / "elements" / f"{name}.png")
            fill = float(el.get("zone_fill", fill_default))
            place = None

            zx0, zy0 = int(zone[0] * W), int(zone[1] * H)
            zx1, zy1 = int(zone[2] * W), int(zone[3] * H)
            zw, zh = max(8, zx1 - zx0), max(8, zy1 - zy0)
            ew, eh = int(crop.shape[1]), int(crop.shape[0])
            zone_sc = min(zw / ew, zh / eh) * fill
            # 按分区期望尺度收窄搜索，禁止扫到「极小假峰」
            sc_lo = max(loc_scale_floor, zone_sc * 0.55)
            sc_hi = min(1.35, zone_sc * 1.25)

            try:
                loc = locate_element(
                    src,
                    crop,
                    scale_min=sc_lo,
                    scale_max=max(sc_lo + 0.05, sc_hi),
                    exclude_mask=exclude,
                    search_zone=zone,  # type: ignore[arg-type]
                )
                ok_conf = loc.confidence >= loc_conf_min
                ok_floor = loc.scale >= loc_scale_floor
                ok_zone = loc.scale >= zone_sc * loc_vs_zone_min
                if ok_conf and ok_floor and ok_zone:
                    place = loc
                else:
                    print(
                        f"  place {name}: reject locate "
                        f"conf={loc.confidence:.2f} sc={loc.scale:.2f} "
                        f"(need conf>={loc_conf_min:.2f} sc>={loc_scale_floor:.2f} "
                        f"and >={zone_sc * loc_vs_zone_min:.2f}=zone*{loc_vs_zone_min:.2f})"
                    )
            except Exception as e:  # noqa: BLE001
                print(f"  place {name}: locate failed ({e})")
            if place is None:
                place = place_to_storyboard_ink(
                    H, W, crop, zone, ink, fill=fill, exclude=exclude  # type: ignore[arg-type]
                )
            if place is None:
                place = place_in_zone(H, W, crop, zone, fill=fill, y_align="center", ink=None)  # type: ignore[arg-type]
                place = Placement(
                    x=place.x,
                    y=place.y,
                    width=place.width,
                    height=place.height,
                    scale=place.scale,
                    confidence=place.confidence,
                    method="zone_fallback_storyboard",
                )
            print(
                f"  place {name}: ({place.x},{place.y}) {place.width}x{place.height} "
                f"sc={place.scale:.2f} conf={place.confidence:.2f} [{place.method}] "
                f"zone_sc≈{zone_sc:.2f}"
            )
            full = place_on_canvas(H, W, crop, place)
            _save(work / f"aligned_{name}.png", full)
            placed.append((name, full))
            canvas = over(canvas, full)
            exclude = mark_exclude_ink(exclude, full, inflate=8)
            report["layers"].append(
                {
                    "id": name,
                    **place.to_dict(),
                    "zone": list(zone),
                    "zone_sc": round(zone_sc, 4),
                }
            )

    _save(work / "composite.png", canvas)
    gap = 40
    if src is None:
        # layout：左=合成分镜，右=同图（无旧总图可对）
        ref_left = canvas
        label_l, label_r = "layout_storyboard", "same"
        panel = np.zeros((H, W * 2 + gap, 3), np.uint8)
        panel[:, :W] = ref_left
        panel[:, W + gap :] = canvas
    else:
        panel = np.zeros((H, W * 2 + gap, 3), np.uint8)
        panel[:, :W] = src
        panel[:, W + gap :] = canvas
        label_l, label_r = ("storyboard", "composite_on_board")
    cv2.putText(panel, label_l, (40, 80), cv2.FONT_HERSHEY_SIMPLEX, 2.0, (230, 230, 230), 4)
    cv2.putText(panel, label_r, (W + gap + 40, 80), cv2.FONT_HERSHEY_SIMPLEX, 1.6, (180, 255, 180), 4)
    _save(work / "compare_final.png", panel)
    (work / "align_report.json").write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    timing["place_s"] = round(time.perf_counter() - t0, 2)
    print(f"  place {timing['place_s']}s mode={place_mode}")
    return placed


def place_in_zone(
    H: int,
    W: int,
    crop: np.ndarray,
    zone: tuple[float, float, float, float],
    *,
    fill: float = 0.90,
    y_align: str = "center",
    ink: np.ndarray | None = None,
) -> Placement:
    """分区兜底（仅 ink 不足时）；默认居中，不贴底。"""
    del ink  # 对齐分镜时用 place_to_storyboard_ink，这里只做几何兜底
    zx0, zy0 = int(zone[0] * W), int(zone[1] * H)
    zx1, zy1 = int(zone[2] * W), int(zone[3] * H)
    zw, zh = max(8, zx1 - zx0), max(8, zy1 - zy0)
    ew, eh = int(crop.shape[1]), int(crop.shape[0])
    sc = min(zw / ew, zh / eh) * float(fill)
    tw, th = max(8, int(round(ew * sc))), max(8, int(round(eh * sc)))
    x = zx0 + (zw - tw) // 2
    y = zy0 + (zh - th) // 2 if y_align != "top" else zy0 + int(zh * 0.04)
    x = int(np.clip(x, 0, max(0, W - tw)))
    y = int(np.clip(y, 0, max(0, H - th)))
    return Placement(x=x, y=y, width=tw, height=th, scale=float(sc), confidence=0.5, method="zone_fallback")


def place_to_storyboard_ink(
    H: int,
    W: int,
    crop: np.ndarray,
    zone: tuple[float, float, float, float],
    ink: np.ndarray,
    *,
    fill: float = 0.96,
    pad: int = 12,
    exclude: np.ndarray | None = None,
) -> Placement | None:
    """用分镜区内墨迹外包盒当目标框，把 crop 等比贴进去（对齐分镜，不信黑底重生坐标）。

    区内多块墨迹时取最大连通域，并避开 exclude，避免整区杂墨把目标框撑歪。
    """
    zx0, zy0 = int(zone[0] * W), int(zone[1] * H)
    zx1, zy1 = int(zone[2] * W), int(zone[3] * H)
    roi = ink[zy0:zy1, zx0:zx1].copy()
    if exclude is not None:
        roi[exclude[zy0:zy1, zx0:zx1] > 0] = 0
    if int((roi > 0).sum()) < 80:
        return None
    nlab, lab, st, _ = cv2.connectedComponentsWithStats((roi > 0).astype(np.uint8), 8)
    best_i, best_a = 0, 0
    for i in range(1, nlab):
        a = int(st[i, cv2.CC_STAT_AREA])
        if a > best_a:
            best_a, best_i = a, i
    if best_i <= 0 or best_a < 80:
        return None
    ys, xs = np.where(lab == best_i)
    bx0 = max(zx0, zx0 + int(xs.min()) - pad)
    by0 = max(zy0, zy0 + int(ys.min()) - pad)
    bx1 = min(zx1, zx0 + int(xs.max()) + 1 + pad)
    by1 = min(zy1, zy0 + int(ys.max()) + 1 + pad)
    tw_box, th_box = bx1 - bx0, by1 - by0
    if tw_box < 40 or th_box < 40:
        return None
    ew, eh = int(crop.shape[1]), int(crop.shape[0])
    sc = min(tw_box / ew, th_box / eh) * float(fill)
    tw, th = max(8, int(round(ew * sc))), max(8, int(round(eh * sc)))
    x = int(np.clip(bx0 + (tw_box - tw) // 2, 0, max(0, W - tw)))
    y = int(np.clip(by0 + (th_box - th) // 2, 0, max(0, H - th)))
    return Placement(
        x=x, y=y, width=tw, height=th, scale=float(sc), confidence=1.0, method="storyboard_ink"
    )


def _hand_out_size(h: int, w: int, cap: int = 1280, align: int = 10) -> tuple[int, int]:
    """与 RegionStreamRenderer 输出尺寸一致（长边 cap，对齐偶数网格）。"""
    scale = cap / max(h, w)
    a = align if align % 2 == 0 else align * 2
    ow = max(a, (int(round(w * scale)) // a) * a)
    oh = max(a, (int(round(h * scale)) // a) * a)
    return oh, ow


def _write_hold_clip(frame_bgr: np.ndarray, duration_ms: int, out_path: Path, fps: int = 30) -> Path:
    """冻帧占位，把绝对时间轴上的缝隙垫平。必须与书写 clip 同分辨率，禁止 4K 冻帧拼 720p。"""
    out_path.parent.mkdir(parents=True, exist_ok=True)
    n = max(1, int(round(duration_ms * fps / 1000.0)))
    h, w = frame_bgr.shape[:2]
    fourcc = cv2.VideoWriter_fourcc(*"mp4v")
    writer = cv2.VideoWriter(str(out_path), fourcc, fps, (w, h))
    if not writer.isOpened():
        raise RuntimeError(f"无法写 hold clip: {out_path}")
    try:
        for _ in range(n):
            writer.write(frame_bgr)
    finally:
        writer.release()
    return out_path


def render_progressive_hand(
    board: np.ndarray,
    layers_rgba: list[tuple[str, np.ndarray]],
    timings: list[tuple[int, int]],
    out_dir: Path,
    *,
    hand_pad: int = 0,
    bare_tip: bool = False,
    concat_full: bool = True,
    independent: bool = False,
    audio_ms: int | None = None,
    lock_speech_timeline: bool = False,
) -> Path:
    from render_stream_whiteboard import RegionStreamRenderer, _build_cfg

    hand_png = REPO / "assets" / "hand-chalk.png"
    accum = board.copy()
    H, W = board.shape[:2]
    clips: list[Path] = []
    out_dir.mkdir(parents=True, exist_ok=True)
    pad = max(0, int(hand_pad))
    fps = 30
    cap_long = 1280
    out_h, out_w = _hand_out_size(H, W, cap=cap_long)

    def _hold_frame(src: np.ndarray) -> np.ndarray:
        if src.shape[0] == out_h and src.shape[1] == out_w:
            return src
        return cv2.resize(src, (out_w, out_h), interpolation=cv2.INTER_AREA)

    # 字级对齐：片长接到下一元素起笔 / 音轨尾；书写只占口播窗
    if lock_speech_timeline and timings and timings[0][0] > 40:
        hold = out_dir / "_hold_lead.mp4"
        clips.append(_write_hold_clip(_hold_frame(accum), int(timings[0][0]), hold, fps=fps))

    for i, ((name, rgba), (start_ms, dur)) in enumerate(zip(layers_rgba, timings)):
        work = out_dir / f"_layer_{i}"
        work.mkdir(parents=True, exist_ok=True)
        (work / "masks").mkdir(exist_ok=True)
        target = over(accum, rgba)
        # 墨迹/手路径只用「纯黑+当前元素」——粉笔底板噪点若进 ink，会被 unclaimed 扩进本层，手会跳全板
        ink_bg = np.zeros((H, W, 3), np.uint8)
        solo = over(ink_bg, rgba)
        mask_full = (rgba[:, :, 3] > 28).astype(np.uint8) * 255
        k = cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (5, 5))
        mask_full = cv2.dilate(mask_full, k, iterations=1)
        ys, xs = np.where(mask_full > 0)
        if ys.size == 0:
            raise RuntimeError(f"{name}: empty mask")
        x0, y0 = int(xs.min()), int(ys.min())
        x1, y1 = int(xs.max()) + 1, int(ys.max()) + 1
        if pad > 0:
            x0 = max(0, x0 - pad // 5)
            y0 = max(0, y0 - pad // 6)
            x1 = min(W, x1 + pad // 4)
            y1 = min(H, y1 + pad // 4)
        mask_box = np.zeros_like(mask_full)
        mask_box[y0:y1, x0:x1] = mask_full[y0:y1, x0:x1]
        mask_full = mask_box
        mask_crop = mask_full[y0:y1, x0:x1]
        solo_clip = solo.copy()
        solo_clip[:y0, :] = 0
        solo_clip[y1:, :] = 0
        solo_clip[:, :x0] = 0
        solo_clip[:, x1:] = 0
        _save(work / "image.png", solo_clip)
        _save(work / "masks" / "m1.png", mask_crop)

        draw_dur = max(400, int(dur))
        if lock_speech_timeline:
            if i + 1 < len(timings):
                next_start = int(timings[i + 1][0])
            elif audio_ms is not None:
                next_start = int(audio_ms)
            else:
                next_start = int(start_ms) + draw_dur
            clip_total = max(draw_dur, next_start - int(start_ms))
        else:
            clip_total = draw_dur + 400

        ann = {
            "sceneId": "annotation",
            "canvas": {"width": W, "height": H},
            "sceneDurationMs": int(clip_total),
            "elements": [
                {
                    "id": "m1",
                    "label": name,
                    "sequence": 1,
                    "type": "structure",
                    "region": {"x": x0, "y": y0, "width": x1 - x0, "height": y1 - y0},
                    "reveal": {
                        "direction": "left_to_right" if i < 3 else "bottom_to_top",
                        "startMs": 0,
                        "durationMs": int(draw_dur),
                        "protectedRegions": [],
                    },
                    "handPath": {
                        "start": [x0 + 20, y0 + 20],
                        "end": [x1 - 20, y1 - 20],
                        "easing": "easeInOut",
                    },
                    "framed": True,
                    "maskFile": "masks/m1.png",
                }
            ],
        }
        (work / "annotation.json").write_text(
            json.dumps(ann, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )
        args = argparse.Namespace(
            fps=fps,
            grid_edge=None,
            ink_weight=None,
            color_weight=0,
            brush_radius=None,
            cap_long_edge=cap_long,
            ink_path="skeleton",
            color_fill="contour-wipe",
            pause="heavy",
            board_style="chalkboard",
        )
        cfg = _build_cfg(args)
        img = _load_bgr(work / "image.png")
        renderer = RegionStreamRenderer(
            img, ann, cfg, None if bare_tip else (hand_png if hand_png.exists() else None), bare_tip, ann_dir=work
        )
        accum_r = cv2.resize(accum, (renderer.out_w, renderer.out_h), interpolation=cv2.INTER_AREA)
        if independent:
            seed = np.zeros((renderer.out_h, renderer.out_w, 3), np.uint8)
            seed[:] = board[0, 0]
            renderer.drawn[...] = seed.astype(np.float32)
        else:
            renderer.drawn[...] = accum_r.astype(np.float32)
        out_pass = work / "draw.mp4"
        print(
            f"  hand-draw {name} start={int(start_ms)}ms draw={draw_dur}ms clip={clip_total}ms "
            f"region=({x0},{y0},{x1 - x0}x{y1 - y0}) lock={lock_speech_timeline}"
        )
        if lock_speech_timeline:
            renderer.render_to(out_pass, clip_total, draw_ahead_ms=0, lock_reveal=True)
        else:
            renderer.render_to(out_pass, clip_total)
        clips.append(out_pass)
        accum = target

    if lock_speech_timeline and audio_ms is not None and timings:
        covered = int(timings[0][0]) if timings else 0
        for j, (st, du) in enumerate(timings):
            if j + 1 < len(timings):
                covered = int(timings[j + 1][0])
            else:
                covered = max(int(st) + max(400, int(du)), covered)
        if int(audio_ms) - covered > 80:
            clips.append(
                _write_hold_clip(
                    _hold_frame(accum),
                    int(audio_ms) - covered,
                    out_dir / "_hold_tail.mp4",
                    fps=fps,
                )
            )

    if not concat_full or len(clips) <= 1:
        return clips[-1] if clips else out_dir / "draw.mp4"

    lst = out_dir / "_concat_list.txt"
    lst.write_text("\n".join(f"file '{p.resolve().as_posix()}'" for p in clips) + "\n", encoding="utf-8")
    draw = out_dir / "draw.mp4"
    # 禁止 concat -c copy：OpenCV mp4v 冻帧/分层尺寸或 timebase 不一致时会竖条乱码
    r = subprocess.run(
        [
            "ffmpeg",
            "-y",
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
            str(lst),
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "18",
            "-pix_fmt",
            "yuv420p",
            str(draw),
        ],
        capture_output=True,
    )
    if r.returncode != 0:
        raise RuntimeError(f"hand concat failed: {(r.stderr or b'')[-400:]!r}")
    lst.unlink(missing_ok=True)
    return draw



def stage_hand(work: Path, shot: Path, cfg: dict, placed: list[tuple[str, np.ndarray]], timing: dict) -> Path:
    t0 = time.perf_counter()
    board = _load_bgr(work / "board.png")
    import sys as _sys

    _sys.path.insert(0, str(REPO))
    from pipeline import resolve_element_draw_timings, shot_audio_path

    audio = shot_audio_path(shot)
    total = audio_ms(audio) if audio else 22000
    els = list(cfg.get("elements") or [])
    timings, src = resolve_element_draw_timings(shot, els, audio_ms=total)
    lock = src == "tts_timeline"
    preview = work / "preview"
    draw = render_progressive_hand(
        board,
        placed,
        timings,
        preview,
        audio_ms=total,
        lock_speech_timeline=lock,
    )
    timing["hand_s"] = round(time.perf_counter() - t0, 2)
    timing["audio_ms"] = total
    timing["timings_ms"] = timings
    timing["timing_source"] = src
    timing["draw_order"] = [el["id"] for el in els]
    print(f"  hand {timing['hand_s']}s src={src} order={timing['draw_order']}")
    return draw


def stage_mux(work: Path, shot: Path, timing: dict) -> Path:
    t0 = time.perf_counter()
    preview = work / "preview"
    import sys as _sys

    _sys.path.insert(0, str(REPO))
    from pipeline import mux, shot_audio_path, speakable_for_tts
    from subtitle_pipeline import make_shot_subtitles

    audio = shot_audio_path(shot)
    if audio is None:
        raise FileNotFoundError(f"缺音轨: {shot}")
    dest_aud = preview / audio.name
    shutil.copy2(audio, dest_aud)
    # 字幕/下游习惯 audio.mp3 名；wav 也直接可用
    if dest_aud.name != "audio.mp3" and not (preview / "audio.mp3").exists():
        shutil.copy2(audio, preview / "audio.mp3")
        mux_audio = preview / "audio.mp3"
    else:
        mux_audio = dest_aud if dest_aud.name == "audio.mp3" else (preview / "audio.mp3" if (preview / "audio.mp3").exists() else dest_aud)

    draw = preview / "draw.mp4"
    muxed = preview / "mux.mp4"
    mux(draw, mux_audio, muxed)
    nar = (shot / "narration.txt").read_text(encoding="utf-8").strip()
    final = preview / "final.mp4"
    make_shot_subtitles(speakable_for_tts(nar), mux_audio, muxed, final, work_dir=preview)
    timing["mux_s"] = round(time.perf_counter() - t0, 2)
    print(f"  mux+subs {timing['mux_s']}s → {final}")
    return final


def main() -> None:
    ap = argparse.ArgumentParser(description="黑底元素 → 定位 → 逐元素手部")
    ap.add_argument("ep", type=Path, help="集目录 out/YYYY-MM-DD")
    ap.add_argument("shot", help="镜号 01/02/...")
    ap.add_argument(
        "--stage",
        default="all",
        help="all | gen | cutout | place | hand | mux | 逗号组合",
    )
    ap.add_argument("--config", type=Path, default=None, help="元素配置 JSON（默认 shot/_blackbg/elements.json）")
    args = ap.parse_args()

    ep = args.ep if args.ep.is_absolute() else REPO / args.ep
    shot = ep / f"{int(args.shot):02d}"
    work = shot / "_blackbg"
    work.mkdir(parents=True, exist_ok=True)
    cfg_path = args.config or (work / "elements.json")
    if not cfg_path.exists():
        raise SystemExit(f"missing config: {cfg_path}")
    cfg = json.loads(cfg_path.read_text(encoding="utf-8"))

    place_mode = cfg.get("place_mode", "storyboard_strict")
    # storyboard_strict / unified：需要元素参考分镜；layout：程序槽位，不强制 compose
    need_compose = place_mode in ("storyboard_strict", "unified", "compose")
    stages = {
        s.strip()
        for s in (
            args.stage.split(",")
            if args.stage != "all"
            else (
                ["gen", "cutout", "compose", "place", "hand", "mux"]
                if need_compose
                else ["gen", "cutout", "place", "hand", "mux"]
            )
        )
    }

    timing: dict = {"shot": shot.name, "stages": sorted(stages)}
    wall0 = time.perf_counter()
    placed: list[tuple[str, np.ndarray]] | None = None

    if "gen" in stages:
        print("[gen]")
        stage_gen(work, shot, cfg, timing)
    if "cutout" in stages:
        print("[cutout]")
        stage_cutout(work, cfg, timing)
    if "compose" in stages or (
        need_compose and "place" in stages and not (work / "storyboard.png").exists()
    ):
        print("[compose]")
        from blackbg_compose import stage_compose

        stage_compose(work, shot, cfg, timing)
    if "place" in stages:
        print("[place]")
        if place_mode == "unified":
            # 旧路径：分镜像素当成片（已弃用，仅兼容）
            from blackbg_compose import stage_place_unified

            placed = stage_place_unified(work, shot, cfg, timing)
        else:
            placed = stage_place(work, shot, cfg, timing)
    if "hand" in stages:
        print("[hand]")
        if placed is None:
            # reload from aligned
            placed = []
            for el in cfg["elements"]:
                name = el["id"]
                placed.append((name, _load_rgba(work / f"aligned_{name}.png")))
            if not (work / "board.png").exists():
                src = _load_bgr(shot / "image.png")
                board_path = REPO / "out/2026-09-17/_pilot_blackbg/board_black_chalk.png"
                _save(work / "board.png", make_board(src.shape[0], src.shape[1], board_path))
        stage_hand(work, shot, cfg, placed, timing)
    if "mux" in stages:
        print("[mux]")
        stage_mux(work, shot, timing)

    timing["total_s"] = round(time.perf_counter() - wall0, 2)
    (work / "timing.json").write_text(json.dumps(timing, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print("TIMING", json.dumps(timing, ensure_ascii=False))


if __name__ == "__main__":
    main()
