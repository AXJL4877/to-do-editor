"""Regressions for retained multilayer preview frames during transform gestures."""

import os
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest
import zlib


ROOT = Path(__file__).resolve().parents[2]


@unittest.skipUnless(os.name == "nt", "Native preview requires Windows")
class VideoStackTests(unittest.TestCase):
    def test_drag_retains_layers_and_does_not_decode_again(self):
        host = Path(os.environ.get("TODO_PREVIEW_HOST",
                                  ROOT / "native/build/bin/preview_host.exe"))
        test_root = ROOT / "target/preview_tests"
        test_root.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="layers-", dir=test_root) as temporary:
            # Small uncompressed BMP, decoded through the production WIC path.
            path = Path(temporary) / "layer.bmp"
            pixels = bytes([40, 100, 220, 0]) * 16 * 16
            path.write_bytes(struct.pack("<2sIHHI", b"BM", 54 + len(pixels), 0, 0, 54)
                             + struct.pack("<IiiHHIIiiII", 40, 16, 16, 1, 32, 0,
                                           len(pixels), 0, 0, 0, 0) + pixels)
            # 3x3 RGBA PNG whose center pixel is half transparent.
            translucent = Path(temporary) / "text.png"
            rows = b"".join(
                bytes([0]) + b"".join(bytes([220, 10, 10, 128 if (x, y) == (1, 1) else 255])
                                      for x in range(3))
                for y in range(3))

            def chunk(kind, body):
                return (struct.pack(">I", len(body)) + kind + body
                        + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF))

            signature = bytes([0x89]) + b"PNG" + bytes([0x0D, 0x0A, 0x1A, 0x0A])
            translucent.write_bytes(signature
                                    + chunk(b"IHDR", struct.pack(">IIBBBBB", 3, 3, 8, 6, 0, 0, 0))
                                    + chunk(b"IDAT", zlib.compress(rows))
                                    + chunk(b"IEND", b""))
            result = subprocess.run(
                [str(host.parent / "video_stack_tests.exe"), str(path), str(translucent)],
                capture_output=True, text=True, timeout=30,
                creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
            )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
