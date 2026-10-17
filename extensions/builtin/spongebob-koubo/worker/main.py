"""Deterministic migration worker for the spongebob-koubo workflow.

The worker owns sequencing and emits editable items progressively. Network work
is always delegated to host services; the offline fixture uses local
placeholder media so package self-checks never need credentials.
"""
import hashlib
import json
import math
import os
import shutil
import wave
from pathlib import Path

from PIL import Image, ImageDraw
from todo_worker import Worker

worker = Worker(version="2.1.0")

_BGM = {
    "intro": "intro-holding-court.mp3",
    "mid": "mid-jingwei.mp3",
    "outro": "outro-memories.mp3",
}


def _rev(*parts):
    return hashlib.sha256("|".join(map(str, parts)).encode("utf-8")).hexdigest()[:16]


def _relative(ctx, path):
    """Workdir-relative protocol path, robust to Windows ``..`` spellings."""
    return os.path.relpath(path, ctx.workdir).replace("\\", "/")


def _script_chunks(script, limit=1200):
    """Split speech at punctuation without ever exceeding the service limit."""
    text = " ".join(str(script or "").split())
    chunks = []
    while text:
        if len(text) <= limit:
            chunks.append(text)
            break
        cut = max(text.rfind(mark, 0, limit + 1) for mark in "。！？；，, ")
        if cut < max(80, limit // 2):
            cut = limit
        chunks.append(text[:cut].strip())
        text = text[cut:].strip()
    return chunks or ["离线样例：海绵宝宝口播"]


def _design_shots(script, design, duration):
    """Normalize chat JSON into a small, deterministic shot plan."""
    value = design.get("text") if isinstance(design, dict) else design
    if isinstance(value, str):
        try:
            value = json.loads(value)
        except (TypeError, ValueError):
            value = None
    candidates = value.get("shots") if isinstance(value, dict) else None
    if not candidates and isinstance(value, dict):
        candidates = value.get("scenes")
    if not isinstance(candidates, list):
        candidates = []
    fallback = _script_chunks(script, 80)
    count = max(1, min(8, len(candidates) or len(fallback)))
    slot = max(0.8, float(duration) / count)
    shots = []
    for index in range(count):
        shot = candidates[index] if index < len(candidates) and isinstance(candidates[index], dict) else {}
        text = str(shot.get("text") or shot.get("label") or (fallback[index] if index < len(fallback) else script))
        draw = str(shot.get("draw") or shot.get("description") or text)
        at = float(shot.get("at", index * slot)) if str(shot.get("at", "")).strip() else index * slot
        at = max(0.0, min(float(duration), at))
        shots.append({
            "index": index,
            "text": text[:120],
            "draw": draw[:1000],
            "at": at,
            "duration": max(0.6, min(slot, float(duration) - at)),
        })
    return shots


def _design_bgm(design, duration):
    """Read optional chat BGM timing without letting malformed plans break a run."""
    defaults = {"intro": 0.0, "mid": float(duration) * 0.35, "outro": float(duration) * 0.75}
    value = design.get("text") if isinstance(design, dict) else design
    if isinstance(value, str):
        try:
            value = json.loads(value)
        except (TypeError, ValueError):
            value = None
    plan = value.get("bgm_plan") if isinstance(value, dict) else None
    if isinstance(plan, dict):
        entries = plan.items()
    elif isinstance(plan, list):
        entries = ((str(item.get("name") or item.get("id") or ""), item)
                   for item in plan if isinstance(item, dict))
    else:
        entries = ()
    for name, item in entries:
        if name not in defaults:
            continue
        raw = item.get("at", item.get("start", item)) if isinstance(item, dict) else item
        try:
            at = float(raw)
        except (TypeError, ValueError):
            continue
        if math.isfinite(at):
            defaults[name] = max(0.0, min(float(duration), at))
    return defaults


def _write_silence(path, seconds, rate=24000):
    path.parent.mkdir(parents=True, exist_ok=True)
    count = max(1, int(round(seconds * rate)))
    with wave.open(str(path), "wb") as out:
        out.setnchannels(1)
        out.setsampwidth(2)
        out.setframerate(rate)
        out.writeframes(b"\0\0" * count)


def _write_wipe_sfx(path, seconds, rate=24000):
    """Deterministic bubble-pop cue; keeps transition audio audible offline."""
    path.parent.mkdir(parents=True, exist_ok=True)
    count = max(1, int(round(seconds * rate)))
    with wave.open(str(path), "wb") as out:
        out.setnchannels(1)
        out.setsampwidth(2)
        out.setframerate(rate)
        samples = bytearray()
        for index in range(count):
            t = index / rate
            envelope = max(0.0, 1.0 - t / max(seconds, 1e-6)) ** 2
            value = int(math.sin(2.0 * math.pi * (320.0 + 900.0 * t) * t) * 5200.0 * envelope)
            samples.extend(int(max(-32768, min(32767, value))).to_bytes(2, "little", signed=True))
        out.writeframes(samples)


def _write_card(path, title, index, width=640, height=360):
    path.parent.mkdir(parents=True, exist_ok=True)
    colors = [(21, 35, 74, 255), (71, 34, 75, 255), (25, 76, 70, 255), (79, 54, 27, 255)]
    image = Image.new("RGBA", (width, height), colors[index % len(colors)])
    draw = ImageDraw.Draw(image)
    for step in range(6):
        x = 20 + step * 96
        draw.rounded_rectangle((x, 48 + step * 5, x + 160, 112 + step * 8), radius=18,
                               outline=(245, 225, 122, 210), width=3)
    draw.text((32, height - 76), title[:28], fill=(255, 255, 255, 255))
    image.save(path, "PNG", optimize=False)


def _write_intro(path, title):
    path.parent.mkdir(parents=True, exist_ok=True)
    image = Image.new("RGBA", (640, 144), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    draw.rounded_rectangle((12, 12, 628, 132), radius=24, fill=(28, 18, 28, 220),
                           outline=(245, 62, 72, 255), width=5)
    draw.text((32, 42), title[:24], fill=(255, 78, 86, 255))
    image.save(path, "PNG", optimize=False)


def _shot_support_items(ctx, index, shot, link):
    """Local prop/dialogue layers keep the workflow editable without more paid calls."""
    prop_file = ctx.path(f"out/shot-{index:02d}-prop.png")
    _write_card(prop_file, f"道具 {shot['text'][:12]}", index + 2, 640, 360)
    bubble_file = ctx.path(f"out/shot-{index:02d}-bubble.png")
    _write_card(bubble_file, f"“{shot['text'][:10]}”", index + 3, 640, 180)
    at = float(shot["at"])
    duration = float(shot["duration"])
    return [
        _item(
            f"shot/{index}/prop",
            _rev("prop", index, shot["text"]),
            "visual",
            2,
            at + min(0.25, duration * 0.2),
            duration * 0.8,
            _relative(ctx, prop_file),
            {"kind": "rise", "duration": min(0.35, duration * 0.35)},
            link=link,
        ),
        _item(
            f"shot/{index}/bubble",
            _rev("bubble", index, shot["text"]),
            "visual",
            3,
            at + min(0.5, duration * 0.35),
            duration * 0.65,
            _relative(ctx, bubble_file),
            {"kind": "pop", "duration": min(0.25, duration * 0.25)},
            link=link,
        ),
    ]


def _bgm_item(ctx, name, at, duration, script):
    """Copy the licensed track into the run workdir and emit one music lane item."""
    package_file = Path(__file__).resolve().parent.parent / "assets" / "bgm" / _BGM[name]
    destination = ctx.path(f"out/bgm-{name}.mp3")
    if package_file.is_file():
        shutil.copyfile(package_file, destination)
    else:
        # Development checkouts without the optional media assets remain
        # deterministic; packaged builds always contain the licensed tracks.
        _write_silence(destination.with_suffix(".wav"), max(0.5, duration - at))
        destination = destination.with_suffix(".wav")
    # Keep each bed on its own audio lane; voice occupies lane 0 and the
    # independent beds must remain mixable when their spans overlap.
    lane = {"intro": 1, "mid": 2, "outro": 3}[name]
    return _item(f"bgm/{name}", _rev("bgm", name, script), "music", lane, at,
                 max(0.5, duration - at),
                 _relative(ctx, destination),
                 gain=0.3, fade={"in": 0.25, "out": 0.5})


def _item(key, rev, role, lane, start, duration, file, intro=None, gain=None, fade=None, link=None):
    value = {
        "key": key,
        "rev": rev,
        "role": role,
        "lane": lane,
        "start": max(0.0, float(start)),
        "duration": max(0.1, float(duration)),
        "source": {"type": "file", "file": file},
    }
    if intro:
        value["intro"] = intro
    if gain is not None:
        value["gain"] = max(0.0, min(4.0, float(gain)))
    if fade:
        value["fade"] = fade
    if link:
        value["link"] = link
    return value


def _write_wipe_frames(ctx, index, title, fps, duration):
    frames_dir = ctx.path(f"out/wipe-{index:02d}.frames")
    frames_dir.mkdir(parents=True, exist_ok=True)
    frame_count = max(2, int(round(fps * duration)))
    for frame in range(frame_count):
        progress = (frame + 1) / frame_count
        image = Image.new("RGBA", (640, 360), (0, 0, 0, 0))
        draw = ImageDraw.Draw(image)
        for bubble in range(7):
            phase = (progress * 1.2 + bubble * 0.13) % 1.0
            radius = 18.0 + phase * 150.0
            x = 320.0 + math.cos(bubble * 1.7) * 160.0 * (1.0 - phase)
            y = 180.0 + math.sin(bubble * 1.3) * 90.0 * (1.0 - phase)
            alpha = int(185.0 * (1.0 - phase) ** 1.4)
            draw.ellipse((x - radius, y - radius, x + radius, y + radius),
                         outline=(245, 225, 122, alpha), width=5)
        image.save(frames_dir / f"{frame + 1:06d}.png", "PNG", optimize=False)
    return frames_dir


def _transition_items(ctx, index, at, duration, title, encode=False):
    """Create a transparent bubble wipe and its independent transition SFX."""
    length = min(0.3, max(0.1, duration))
    if encode:
        fps = max(12, min(60, int(round(float((ctx.canvas or {}).get("fps", 30))))))
        frames_dir = _write_wipe_frames(ctx, index, title, fps, length)
        encoded = ctx.call("encode", {
            "frames": _relative(ctx, frames_dir),
            "fps": fps,
            "alpha": True,
            "quality": "standard",
        })
        wipe_file = encoded.get("file") if isinstance(encoded, dict) else None
        if not wipe_file:
            raise RuntimeError("透明泡泡转场编码没有返回 file")
    else:
        wipe = ctx.path(f"out/wipe-{index:02d}.png")
        _write_card(wipe, f"泡泡转场 {title[:12]}", index + 5, 640, 360)
        wipe_file = _relative(ctx, wipe)
    sfx = ctx.path(f"out/wipe-{index:02d}.wav")
    _write_wipe_sfx(sfx, length)
    start = max(0.0, float(at) - length)
    return [
        _item(
            f"shot/{index}/wipe",
            _rev("wipe", index, title),
            "visual",
            4,
            start,
            length,
            wipe_file,
            None if encode else {"kind": "stamp", "duration": length},
            link=f"shot/{index}/character",
        ),
        _item(
            f"shot/{index}/wipe/sfx",
            _rev("wipe-sfx", index, title),
            "sfx",
            4,
            start,
            length,
            _relative(ctx, sfx),
            gain=0.7,
            fade={"in": 0.02, "out": min(0.1, length / 2)},
            link=f"shot/{index}/character",
        ),
    ]


def _subtitle_items(chunk_index, offset, cues):
    """Turn host subtitle cues into editable text items with stable keys."""
    output = []
    cue_list = cues.get("cues") if isinstance(cues, dict) else None
    if not isinstance(cue_list, list):
        return output
    for cue_index, cue in enumerate(cue_list):
        if not isinstance(cue, dict):
            continue
        text = str(cue.get("text") or "").strip()
        if not text:
            continue
        start = max(0.0, float(cue.get("start", 0.0)))
        end = max(start + 0.05, float(cue.get("end", start + 0.2)))
        output.append({
            "key": f"sub/{chunk_index}/{cue_index}",
            "rev": _rev("sub", chunk_index, cue_index, text, cue),
            "role": "subtitle",
            "lane": 0,
            "start": round(offset + start, 4),
            "duration": round(end - start, 4),
            "source": {"type": "text", "content": {
                "text": text,
                "styleId": "grunge-red-echo",
            }},
        })
    return output


def _offline(ctx, script):
    out = ctx.path("out")
    duration = max(4.0, min(120.0, 1.4 + len(script) / 55.0))
    shot_count = max(1, min(6, math.ceil(len(script or "离线样例") / 80)))
    shot_duration = duration / shot_count
    items = []
    intro_file = out / "intro-red.png"
    _write_intro(intro_file, "海绵宝宝口播")
    items.append(_item("intro_red", _rev("intro", script), "visual", 5, 0.0,
                       min(1.0, duration), _relative(ctx, intro_file),
                       {"kind": "fade", "duration": min(0.35, duration)}))
    for index in range(shot_count):
        title = (script or "海绵宝宝口播")[(index * 80):((index + 1) * 80)] or "海绵宝宝口播"
        voice_file = out / f"voice-{index:02d}.wav"
        _write_silence(voice_file, shot_duration)
        items.append(_item(f"voice/{index}", _rev("voice", index, script), "voice", 0,
                           index * shot_duration, shot_duration,
                           _relative(ctx, voice_file)))
        image_file = out / f"shot-{index:02d}.png"
        _write_card(image_file, title, index)
        items.append(_item(f"shot/{index}/character", _rev("shot", index, title), "visual", 1,
                           index * shot_duration, shot_duration,
                           _relative(ctx, image_file),
                           {"kind": "fade", "duration": min(0.4, shot_duration)}))
        items.extend(_shot_support_items(ctx, index, {
            "text": title,
            "at": index * shot_duration,
            "duration": shot_duration,
        }, f"shot/{index}/character"))
        if index:
            items.extend(_transition_items(ctx, index, index * shot_duration, shot_duration, title))
        subtitle_file = out / f"sub-{index:02d}.png"
        _write_card(subtitle_file, title[:18], index + 1, 640, 96)
        items.append(_item(f"sub/{index}", _rev("sub", index, title), "subtitle", 0,
                           index * shot_duration, shot_duration,
                           _relative(ctx, subtitle_file),
                           {"kind": "fade", "duration": min(0.2, shot_duration)}))
        ctx.progress("离线镜头", (index + 1) / shot_count * 0.8)
    for name, at in (("intro", 0.0), ("mid", duration * 0.35), ("outro", duration * 0.75)):
        items.append(_bgm_item(ctx, name, at, duration, script))
    ctx.emit(items)
    return {
        "gates": {"g1": "pass", "g3": "pass", "g5": "pass", "g8": "pass"},
        "items": len(items),
        "duration": duration,
        "offline": True,
    }


@worker.op("run")
def run(ctx):
    value = ctx.input if isinstance(ctx.input, dict) else {}
    script = str(value.get("script", ""))[:12000]
    ctx.progress("准备工作流", 0.05)
    if value.get("offline") or not script:
        return _offline(ctx, script or "离线样例：海绵宝宝口播")

    # Online calls are host-owned and durable. No retry or subprocess fallback
    # lives in this package. Speech is chunked so long scripts get one editable
    # voice item per paragraph instead of silently truncating at 1200 chars.
    chunks = _script_chunks(script)
    voice_items = []
    voice_results = []
    offset = 0.0
    for index, chunk in enumerate(chunks):
        voice = ctx.call("speech", {
            "text": chunk,
            "voice": ctx.params.get("voice", "zh_female_mizai_uranus_bigtts"),
        })
        voice_results.append((chunk, voice))
        voice_file = voice.get("file") if isinstance(voice, dict) else None
        voice_duration = float(voice.get("duration", 1.0)) if isinstance(voice, dict) else 1.0
        if voice_file:
            voice_items.append(_item(f"voice/{index}", _rev("voice", index, chunk), "voice", 0,
                                     offset, voice_duration, voice_file))
        offset += max(0.1, voice_duration)
        ctx.progress("配音", 0.08 + 0.22 * (index + 1) / len(chunks))
    duration = max(0.6, offset)
    if voice_items:
        ctx.emit(voice_items)

    design = ctx.call("chat", {
        "messages": [{"role": "user", "content": script}],
        "json": True,
    })
    shots = _design_shots(script, design, duration)
    bgm_at = _design_bgm(design, duration)
    intro_file = ctx.path("out/intro-red.png")
    _write_intro(intro_file, "海绵宝宝口播")
    ctx.emit([_item("intro_red", _rev("intro", script), "visual", 5, 0.0,
                    min(1.0, duration), _relative(ctx, intro_file),
                    {"kind": "fade", "duration": min(0.35, duration)})])
    visual_count = 0
    for shot in shots:
        index = shot["index"]
        image = ctx.call("image", {
            "prompt": shot["draw"],
            "size": "1024x1024",
            "transparent": True,
            "variant": index,
        })
        image_file = image.get("file") if isinstance(image, dict) else None
        if image_file:
            shot_items = [_item(
                f"shot/{index}/character",
                _rev("shot", index, shot["draw"], design),
                "visual",
                1,
                shot["at"],
                shot["duration"],
                image_file,
                {"kind": "fade" if index % 2 == 0 else "pop", "duration": min(0.4, shot["duration"])},
            )]
            shot_items.extend(_shot_support_items(ctx, index, shot, f"shot/{index}/character"))
            if index:
                shot_items.extend(_transition_items(ctx, index, shot["at"], shot["duration"], shot["text"], encode=True))
            ctx.emit(shot_items)
            visual_count += len(shot_items)
        ctx.progress("镜头素材", 0.3 + 0.45 * (index + 1) / len(shots))

    subtitle_items = []
    subtitle_offset = 0.0
    for index, (chunk, voice) in enumerate(voice_results):
        voice_file = voice.get("file") if isinstance(voice, dict) else None
        if not voice_file:
            continue
        voice_duration = float(voice.get("duration", 1.0)) if isinstance(voice, dict) else 1.0
        cues = ctx.call("subtitles", {"script": chunk, "speech": voice_file})
        subtitle_items.extend(_subtitle_items(index, subtitle_offset, cues))
        subtitle_offset += voice_duration
    bgm_items = [
        _bgm_item(ctx, "intro", bgm_at["intro"], duration, script),
        _bgm_item(ctx, "mid", bgm_at["mid"], duration, script),
        _bgm_item(ctx, "outro", bgm_at["outro"], duration, script),
    ]
    if subtitle_items or bgm_items:
        ctx.emit(subtitle_items + bgm_items)
    return {
        "gates": {"g1": "pass" if len(shots) >= 2 else "warn", "g3": "pass",
                   "g5": "pass", "g8": "pass"},
        "items": len(voice_items) + visual_count + len(subtitle_items) + len(bgm_items),
        "shots": len(shots),
        "duration": duration,
        "bgm_plan": bgm_at,
    }


worker.serve()
