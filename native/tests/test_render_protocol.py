"""Offline export renderer: same layer protocol, decoders and shaders as the
window preview, drawn offscreen; one BGRA frame per committed layer set."""
import os
import struct
import subprocess
import unittest
import zlib
from pathlib import Path
from tempfile import TemporaryDirectory

ROOT = Path(__file__).resolve().parents[2]


def png(path, width, height, rgba_rows, text=None):
    """Tiny RGBA8 PNG writer; `text` adds an iTXt chunk (keyword, value)."""
    raw = b"".join(b"\x00" + bytes(row) for row in rgba_rows)

    def chunk(kind, payload):
        return (struct.pack(">I", len(payload)) + kind + payload +
                struct.pack(">I", zlib.crc32(kind + payload) & 0xffffffff))

    body = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
    if text:
        keyword, value = text
        body += chunk(b"iTXt", keyword.encode() + b"\x00\x00\x00\x00\x00" + value.encode())
    body += chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")
    path.write_bytes(body)


@unittest.skipUnless(os.name == "nt", "Native renderer requires Windows")
class RenderProtocolTests(unittest.TestCase):
    def host(self):
        candidate = Path(os.environ.get("TODO_PREVIEW_HOST", ROOT / "native/build/bin/preview_host.exe"))
        if not candidate.is_file():
            self.skipTest("preview host is not built")
        return candidate.resolve()

    def render(self, payload, width, height):
        process = subprocess.Popen([str(self.host()), "--render"], cwd=self.host().parent,
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        stdout, stderr = process.communicate(payload.encode("utf-8"), timeout=20)
        self.assertEqual(process.returncode, 0, stderr.decode(errors="replace"))
        return stdout

    @staticmethod
    def pixel(frame, width, x, y):
        at = (y * width + x) * 4
        return tuple(frame[at:at + 4])

    def test_empty_frames_are_black(self):
        out = self.render("begin 4 4\nlayers-begin\nlayers-commit\nlayers-begin\nlayers-commit\nend\n", 4, 4)
        self.assertEqual(len(out), 4 * 4 * 4 * 2)
        self.assertEqual(self.pixel(out, 4, 1, 1)[:3], (0, 0, 0))

    def test_image_layer_is_fitted_and_positioned_like_the_preview(self):
        with TemporaryDirectory() as directory:
            red = Path(directory) / "red.png"
            png(red, 2, 2, [(255, 0, 0, 255) * 2] * 2)
            # Half-size square centered on a 64x64 canvas, shifted right by 1/4.
            out = self.render(f"begin 64 64\nlayers-begin\nlayer image 0 0.25 0 0.5 1 1 1 {red}\n"
                              "layers-commit\nend\n", 64, 64)
            self.assertEqual(len(out), 64 * 64 * 4)
            self.assertGreater(self.pixel(out, 64, 48, 32)[2], 200, "inside the moved square is red")
            self.assertEqual(self.pixel(out, 64, 16, 32)[:3], (0, 0, 0), "outside stays black")

    def test_reveal_plays_by_progress(self):
        with TemporaryDirectory() as directory:
            atlas = Path(directory) / "e.reveal.png"
            # 2x1 picture (white, white) + times (0.25, 0.75).
            row = [255, 255, 255, 255, 255, 255, 255, 255,
                   64, 0, 0, 255, 191, 255, 0, 255]
            png(atlas, 4, 1, [row], ("todo-reveal", '{"spec":"todo-reveal/1","width":2,"height":1,'
                                                    '"duration":2.0,"softMax":0.0}'))
            frames = self.render(f"begin 64 32\n"
                                 f"layers-begin\nlayer reveal:0 0.5 0 0 1 1 1 1 {atlas}\nlayers-commit\n"
                                 f"layers-begin\nlayer reveal:0 1 0 0 1 1 1 1 {atlas}\nlayers-commit\nend\n", 64, 32)
            size = 64 * 32 * 4
            half, full = frames[:size], frames[size:]
            self.assertGreater(self.pixel(half, 64, 8, 16)[0], 200, "early pixel is drawn halfway")
            self.assertLess(self.pixel(half, 64, 56, 16)[0], 30, "late pixel is not drawn yet")
            self.assertGreater(self.pixel(full, 64, 56, 16)[0], 200, "everything is drawn at the end")

    def test_missing_source_fails_with_a_reason(self):
        process = subprocess.Popen([str(self.host()), "--render"], cwd=self.host().parent,
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        _, stderr = process.communicate(b"begin 4 4\nlayers-begin\nlayer image 0 0 0 1 1 1 1 Z:\\missing.png\n"
                                        b"layers-commit\nend\n", timeout=20)
        self.assertNotEqual(process.returncode, 0)
        self.assertIn(b"cannot open layer", stderr)


if __name__ == "__main__":
    unittest.main()
