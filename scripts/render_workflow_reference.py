"""Render a deterministic five-time contact sheet from a saved workflow document.

This creates a current-workflow baseline for later comparison with the legacy
pipeline. It deliberately does not claim visual equivalence without legacy
frames.
"""

import argparse
import json
import struct
import subprocess
from pathlib import Path

from PIL import Image, ImageDraw


def read_exact(stream, count):
    data = bytearray()
    while len(data) < count:
        block = stream.read(count - len(data))
        if not block:
            raise RuntimeError("preview host closed before a complete frame")
        data.extend(block)
    return bytes(data)


def render(host, workdir, items, width, height, output):
    visual = [
        item for item in items
        if item.get("role") in {"visual", "subtitle"}
        and item.get("source", {}).get("type") == "file"
    ]
    end = max((float(item.get("start", 0)) + float(item.get("duration", 0))
               for item in visual), default=1.0)
    times = [round(end * fraction, 4) for fraction in (0.0, 0.25, 0.5, 0.75, 1.0)]
    frame_bytes = width * height * 4
    process = subprocess.Popen(
        [str(host), "--render"], cwd=host.parent,
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    )
    frames = []
    try:
        process.stdin.write(f"begin {width} {height}\n".encode())
        process.stdin.flush()
        for time in times:
            active = [
                item for item in visual
                if float(item.get("start", 0)) <= time <
                float(item.get("start", 0)) + float(item.get("duration", 0))
            ]
            process.stdin.write(b"layers-begin\n")
            for item in sorted(active, key=lambda value: (value.get("lane", 0), value["key"])):
                path = (workdir / item["source"]["file"]).resolve()
                process.stdin.write(
                    f"layer image 0 0 0 1 1 1 1 {path}\n".encode()
                )
            process.stdin.write(b"layers-commit\n")
            process.stdin.flush()
            raw = read_exact(process.stdout, frame_bytes)
            rgba = bytearray(frame_bytes)
            for index in range(0, frame_bytes, 4):
                rgba[index:index + 4] = bytes((
                    raw[index + 2], raw[index + 1], raw[index], raw[index + 3]
                ))
            frames.append(Image.frombytes("RGBA", (width, height), bytes(rgba)))
        process.stdin.write(b"end\n")
        process.stdin.close()
        process.wait(timeout=30)
        if process.returncode != 0:
            raise RuntimeError(process.stderr.read().decode(errors="replace"))
    except BaseException:
        process.kill()
        process.wait()
        raise

    tile_w, tile_h = 320, 180
    sheet = Image.new("RGB", (tile_w * len(frames), tile_h + 28), (24, 24, 28))
    draw = ImageDraw.Draw(sheet)
    frame_paths = []
    for index, frame in enumerate(frames):
        frame_path = output.with_name(f"{output.stem}-{index + 1:02d}.png")
        frame.save(frame_path, "PNG", optimize=False)
        frame_paths.append(str(frame_path))
        tile = frame.convert("RGB").resize((tile_w, tile_h), Image.Resampling.LANCZOS)
        sheet.paste(tile, (index * tile_w, 0))
        draw.text((index * tile_w + 8, tile_h + 6), f"t={times[index]:.2f}s", fill=(240, 240, 240))
    output.parent.mkdir(parents=True, exist_ok=True)
    sheet.save(output, "PNG", optimize=False)
    return times, [item["key"] for item in visual]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", type=Path, required=True)
    parser.add_argument("--workdir", type=Path, required=True)
    parser.add_argument("--production", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--width", type=int, default=960)
    parser.add_argument("--height", type=int, default=540)
    args = parser.parse_args()
    document = json.loads(args.production.read_text(encoding="utf-8"))
    items = document["workflow"]["items"]
    times, keys = render(
        args.host.resolve(), args.workdir.resolve(), items,
        args.width, args.height, args.output.resolve()
    )
    args.output.with_suffix(".json").write_text(
        json.dumps({
            "source": str(args.production.resolve()),
            "times": times,
            "visualKeys": keys,
            "frames": frame_paths,
            "legacyComparison": "pending legacy screenshots",
        }, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
