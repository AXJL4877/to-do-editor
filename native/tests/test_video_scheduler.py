"""Real decoder scheduling: cold progress, independent sources and final coherence."""

import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0)


@unittest.skipUnless(os.name == "nt", "Native scheduler requires Windows")
class VideoSchedulerTests(unittest.TestCase):
    def test_independent_streams_publish_progress_and_finish_coherently(self):
        host = Path(os.environ.get("TODO_PREVIEW_HOST",
                                  ROOT / "native/build/bin/preview_host.exe"))
        ffmpeg = ROOT / "dependencies/ffmpeg/bin/ffmpeg.exe"
        if not ffmpeg.is_file():
            ffmpeg = shutil.which("ffmpeg")
        self.assertTrue(ffmpeg, "FFmpeg is required to generate the long-GOP fixture")
        test_root = ROOT / "target/preview_tests"
        test_root.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="scheduler-", dir=test_root) as temporary:
            still = Path(temporary) / "layer.bmp"
            pixels = bytes([40, 100, 220, 0]) * 16 * 16
            still.write_bytes(struct.pack("<2sIHHI", b"BM", 54 + len(pixels), 0, 0, 54)
                              + struct.pack("<IiiHHIIiiII", 40, 16, 16, 1, 32, 0,
                                            len(pixels), 0, 0, 0, 0) + pixels)
            video = Path(temporary) / "long-gop.mp4"
            generated = subprocess.run(
                [str(ffmpeg), "-v", "error", "-f", "lavfi", "-i",
                 "testsrc2=size=1920x1080:rate=30", "-t", "10", "-c:v", "libx264",
                 "-preset", "ultrafast", "-g", "250", "-keyint_min", "250",
                 "-sc_threshold", "0", "-pix_fmt", "yuv420p", str(video)],
                capture_output=True, timeout=60, creationflags=NO_WINDOW,
            )
            self.assertEqual(generated.returncode, 0, generated.stderr.decode("utf-8", "replace"))
            result = subprocess.run(
                [str(host.parent / "video_scheduler_tests.exe"), str(video), str(still)],
                capture_output=True, text=True, timeout=60, creationflags=NO_WINDOW,
            )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
