"""Measure the offline render protocol at a reproducible 1080p workload.

This is intentionally a small, dependency-free benchmark. It streams the
BGRA response instead of collecting all frames, so the measurement includes
the same readback cost as ``OfflineRenderer`` without requiring Pillow or
FFmpeg. The generated still layers exercise the compositing and decoder
reuse path used by the production export fixtures.

Example::

    python native/tests/benchmark_render.py --host native/build/bin/preview_host.exe \
        --layers 3 --frames 30 --output target/render-benchmark.json
"""

import argparse
import hashlib
import json
import struct
import subprocess
import tempfile
import time
import zlib
from pathlib import Path


def _chunk(kind, payload):
    return (struct.pack(">I", len(payload)) + kind + payload +
            struct.pack(">I", zlib.crc32(kind + payload) & 0xffffffff))


def _write_png(path, width, height, color):
    row = bytes((0, *color)) * width
    body = (b"\x89PNG\r\n\x1a\n" +
            _chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)) +
            _chunk(b"IDAT", zlib.compress(row * height, 1)) +
            _chunk(b"IEND", b""))
    path.write_bytes(body)


def _read_exact(stream, size):
    remaining = size
    while remaining:
        block = stream.read(min(1024 * 1024, remaining))
        if not block:
            raise RuntimeError("offline renderer closed stdout before a complete frame")
        remaining -= len(block)


def benchmark(host, width, height, layers, frames):
    frame_bytes = width * height * 4
    with tempfile.TemporaryDirectory(prefix="todo-render-bench-") as directory:
        media = []
        for index in range(layers):
            path = Path(directory) / f"layer-{index}.png"
            _write_png(path, width, height, (32 + index * 48, 88, 168))
            media.append(path)

        process = subprocess.Popen(
            [str(host), "--render"], cwd=host.parent,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            process.stdin.write(f"begin {width} {height}\n".encode())
            process.stdin.flush()
            started = time.perf_counter()
            for _ in range(frames):
                commands = ["layers-begin"]
                commands.extend(
                    f"layer image 0 {index * 0.05:.2f} 0 1 1 1 1 {path}"
                    for index, path in enumerate(media))
                commands.append("layers-commit")
                process.stdin.write(("\n".join(commands) + "\n").encode())
                process.stdin.flush()
                _read_exact(process.stdout, frame_bytes)
            elapsed = time.perf_counter() - started
            process.stdin.write(b"end\n")
            process.stdin.close()
            process.wait(timeout=20)
            stderr = process.stderr.read().decode(errors="replace")
            if process.returncode != 0:
                raise RuntimeError(f"renderer exited {process.returncode}: {stderr.strip()}")
        except BaseException:
            process.kill()
            process.wait()
            raise

    fps = frames / elapsed
    return {
        "host": str(host),
        "host_sha256": hashlib.sha256(host.read_bytes()).hexdigest(),
        "width": width,
        "height": height,
        "layers": layers,
        "frames": frames,
        "elapsed_seconds": round(elapsed, 6),
        "fps": round(fps, 3),
        "realtime_x_at_30fps": round(fps / 30.0, 3),
        "target_realtime_x": 0.8,
        "passed": fps / 30.0 >= 0.8,
        "method": "streamed BGRA readback of generated PNG still layers; includes decoder open and GPU readback",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", type=Path, required=True)
    parser.add_argument("--width", type=int, default=1920)
    parser.add_argument("--height", type=int, default=1080)
    parser.add_argument("--layers", type=int, default=3)
    parser.add_argument("--frames", type=int, default=30)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if min(args.width, args.height, args.layers, args.frames) <= 0:
        parser.error("width, height, layers and frames must be positive")
    host = args.host.resolve(strict=True)
    result = benchmark(host, args.width, args.height, args.layers, args.frames)
    print(json.dumps(result, indent=2))
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
