"""The real host composes a full stack of still layers (todo_core::MAX_PREVIEW_LAYERS)."""

import os
from pathlib import Path
import struct
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_preview import ROOT, Host  # noqa: E402

MAX_PREVIEW_LAYERS = 16


def bmp(path, blue, green, red):
    pixels = bytes([blue, green, red, 0]) * 16 * 16
    path.write_bytes(struct.pack("<2sIHHI", b"BM", 54 + len(pixels), 0, 0, 54)
                     + struct.pack("<IiiHHIIiiII", 40, 16, 16, 1, 32, 0,
                                   len(pixels), 0, 0, 0, 0) + pixels)


@unittest.skipUnless(os.name == "nt", "Native preview requires Windows")
class LayerBudgetTests(unittest.TestCase):
    def test_full_still_stack_is_composed_exactly(self):
        test_root = ROOT / "target/preview_tests"
        test_root.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="budget-", dir=test_root) as temporary:
            paths = []
            for index in range(MAX_PREVIEW_LAYERS + 1):
                path = Path(temporary) / f"layer {index}.bmp"
                bmp(path, index * 10, 100, 200 - index * 10)
                paths.append(path)
            host = Host()
            self.addCleanup(host.close)
            host.send("size-screen 0 0 320 180 0")
            # One layer over the budget: the host keeps the first MAX_PREVIEW_LAYERS.
            lines = ["layers-begin"]
            lines += [f"layer image 0 0 0 0.5 1 1 1 {path}" for path in paths]
            lines.append("layers-commit")
            host.send("\n".join(lines))
            _, frame = host.wait(lambda p: len(p) == 5 and p[0] == "video-frame", timeout=10)
            self.assertEqual(float(frame[2]), 0.0)
            host.send("transport 10010 0")
            host.clock(10010)
            self.assertFalse(host.errors)


if __name__ == "__main__":
    unittest.main()
