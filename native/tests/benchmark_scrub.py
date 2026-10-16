"""Reproducible hidden-host telemetry benchmark; not a physical display FPS test.

python native/tests/benchmark_scrub.py --host native/build/scrub-next/preview_host.exe \
  --media data/untitled/media/VIDEO.mp4 --seconds 2 --fps 60 --output target/result.json

The unchanged host protocol describes the top available layer only. For multiple
layers, error/gap metrics therefore characterize that layer, not every layer or
the DWM/display. Approximate events and exact completions remain distinct.
"""

import argparse
import bisect
import hashlib
import json
import math
import os
from pathlib import Path
import statistics
import sys
import time

from test_preview import Host


def percentile(values, fraction):
    if not values:
        return None
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, math.ceil(fraction * len(ordered)) - 1)]


def commands(media, layers, target):
    if layers == 1:
        return f"seek {target:.9f}"
    return ("layers-begin\n"
            f"layer {target - 0.1:.9f} 0 0 1 1 1 1 {media}\n"
            f"layer {target:.9f} 0.1 0 0.8 1 1 0.65 {media}\n"
            "layers-commit")


def target_at(pattern, index, count, low, high):
    progress = index / max(1, count - 1)
    if pattern == "forward":
        return low + (high - low) * progress
    if pattern == "reverse":
        return high - (high - low) * progress
    # Alternate distant targets while each side still moves, stressing stale
    # seek cancellation and ensuring the final target is unique.
    return low + (high - low) * (0.1 + 0.1 * progress if index % 2 else 0.9 - 0.1 * progress)


def run_case(args, layers, pattern):
    host = Host()
    try:
        host.send("size-screen 0 0 640 360 0")
        initial = target_at(pattern, 0, 2, args.start, args.end)
        if layers == 1:
            host.send(f"path {args.media}\nseek {initial:.9f}")
        else:
            host.send(commands(args.media, layers, initial))
        host.frame(initial, timeout=15)
        count = max(2, round(args.seconds * args.fps))
        sent = []
        start = time.perf_counter()
        for index in range(count):
            target = target_at(pattern, index, count, args.start, args.end)
            stamp = time.perf_counter()
            sent.append((stamp, target))
            host.send(commands(args.media, layers, target))
            delay = start + (index + 1) / args.fps - time.perf_counter()
            if delay > 0:
                time.sleep(delay)
        stopped = time.perf_counter()
        # Unique final target: an earlier exact completion cannot satisfy it.
        final_target = args.start + (args.end - args.start) * 0.371234567
        final_sent = time.perf_counter()
        host.send(commands(args.media, layers, final_target))
        final_stamp, final_parts = host.wait(
            lambda p: len(p) == 5 and p[0] == "video-frame"
            and abs(float(p[2]) - final_target) < 1e-6, timeout=15)

        events = sorted(host.frames + host.approximations)
        during = [(stamp, p) for stamp, p in events if start <= stamp < stopped]
        # Exact/approximate phase changes can republish the same GPU image. Count
        # timestamp changes once so diagnostic throughput cannot inflate updates.
        updates = []
        last_image = None
        target_errors = []
        centers = []
        sent_times = [stamp for stamp, _ in sent]
        for stamp, parts in during:
            timestamp, duration = float(parts[3]), float(parts[4])
            image = (timestamp, duration)
            if image != last_image:
                updates.append(stamp)
                last_image = image
            input_index = max(0, bisect.bisect_right(sent_times, stamp) - 1)
            target = sent[input_index][1]
            target_errors.append(max(timestamp - target, target - timestamp - duration, 0))
            centers.append(abs(timestamp + duration / 2 - target))
        boundaries = [start] + updates + [stopped]
        gaps = [(right - left) * 1000 for left, right in zip(boundaries, boundaries[1:])]
        final_ts, final_duration = float(final_parts[3]), float(final_parts[4])
        result = {
            "layers": layers,
            "pattern": pattern,
            "input_count": count,
            "input_fps": count / (stopped - start),
            "exact_events": sum(parts[0] == "video-frame" for _, parts in during),
            "approximate_events": sum(parts[0] == "video-approx" for _, parts in during),
            "distinct_top_frame_updates": len(updates),
            "distinct_top_frame_updates_per_second": len(updates) / (stopped - start),
            "top_update_gap_p95_ms": percentile(gaps, 0.95),
            "top_max_stall_ms": max(gaps),
            "top_target_interval_error_p50_seconds": percentile(target_errors, 0.50),
            "top_target_interval_error_p95_seconds": percentile(target_errors, 0.95),
            "top_target_interval_error_max_seconds": max(target_errors, default=None),
            "top_target_center_error_p95_seconds": percentile(centers, 0.95),
            "final_exact_latency_ms": (final_stamp - final_sent) * 1000,
            "final_exact_target": final_target,
            "final_exact_interval_error_seconds": max(final_ts - final_target,
                                                       final_target - final_ts - final_duration, 0),
            "stderr": list(host.errors),
        }
        print(json.dumps(result), flush=True)
        return result
    finally:
        host.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", type=Path, required=True)
    parser.add_argument("--media", type=Path, required=True)
    parser.add_argument("--seconds", type=float, default=2)
    parser.add_argument("--fps", type=float, default=60)
    parser.add_argument("--start", type=float, default=0.2)
    parser.add_argument("--end", type=float, default=7.9)
    parser.add_argument("--layers", type=int, nargs="+", default=[1, 2], choices=[1, 2])
    parser.add_argument("--patterns", nargs="+", default=["forward", "reverse", "jumps"],
                        choices=["forward", "reverse", "jumps"])
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    args.host = args.host.resolve(strict=True)
    args.media = args.media.resolve(strict=True)
    if args.seconds <= 0 or args.fps <= 0 or args.start < 0.1 or args.end <= args.start:
        parser.error("seconds/fps must be positive and 0.1 <= start < end")
    os.environ["TODO_PREVIEW_HOST"] = str(args.host)
    report = {
        "host": str(args.host),
        "host_sha256": hashlib.sha256(args.host.read_bytes()).hexdigest(),
        "media": str(args.media),
        "methodology": "Hidden 640x360 MF/D3D host; fixed-rate input; top-available-layer protocol telemetry. "
                       "Input FPS and diagnostic event rates are not physical presentation FPS. "
                       "Distinct updates count timestamp/duration changes, approximate and exact combined. "
                       "Gap metrics include start/end stalls; error is distance of current input target "
                       "from the displayed frame interval. Per-layer errors below the top are unavailable. "
                       "Each case uses a fresh host and warms only its first target, so cache history differs "
                       "naturally by direction. Final exact means a coherent complete stack in the new host.",
        "parameters": {"seconds": args.seconds, "fps": args.fps, "start": args.start, "end": args.end},
        "cases": [],
    }
    for layers in args.layers:
        for pattern in args.patterns:
            report["cases"].append(run_case(args, layers, pattern))
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
