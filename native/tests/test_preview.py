"""Protocol regressions for the real MF/D3D11 preview host (Windows).

Run after native/build.cmd: python -m unittest discover -s native/tests -v
TODO_TEST_MEDIA may point to an existing long-GOP video; otherwise a temporary
1080p H.264 fixture is generated with dependencies/ffmpeg/bin/ffmpeg.exe or FFmpeg on PATH.
"""

import os
import ctypes
from ctypes import wintypes
from pathlib import Path
import queue
import shutil
import subprocess
import tempfile
import threading
import time
import unittest


ROOT = Path(__file__).resolve().parents[2]
NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0)


class Host:
    def __init__(self):
        exe = Path(os.environ.get("TODO_PREVIEW_HOST", ROOT / "native/build/bin/preview_host.exe")).resolve()
        self.process = subprocess.Popen(
            [str(exe)], cwd=exe.parent, stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, creationflags=NO_WINDOW,
        )
        self.lines = queue.Queue()
        self.frames = []
        self.approximations = []
        self.errors = []
        self.readers = [
            threading.Thread(target=self._stdout, daemon=True),
            threading.Thread(target=self._stderr, daemon=True),
        ]
        for reader in self.readers:
            reader.start()

    def _stdout(self):
        for line in self.process.stdout:
            event = (time.perf_counter(), line.decode("utf-8", "replace").split())
            if event[1] and event[1][0] == "video-frame":
                self.frames.append(event)
            elif event[1] and event[1][0] == "video-approx":
                self.approximations.append(event)
            self.lines.put(event)

    def _stderr(self):
        for line in self.process.stderr:
            self.errors.append(line.decode("utf-8", "replace").strip())

    def send(self, text):
        self.process.stdin.write((text + "\n").encode("utf-8"))
        self.process.stdin.flush()

    def wait(self, predicate, timeout=3):
        deadline = time.perf_counter() + timeout
        while time.perf_counter() < deadline:
            try:
                stamp, parts = self.lines.get(timeout=max(0.001, deadline - time.perf_counter()))
            except queue.Empty:
                break
            if predicate(parts):
                return stamp, parts
        raise AssertionError(f"Preview response timed out; stderr: {self.errors}")

    def clock(self, marker, timeout=3):
        return self.wait(lambda p: len(p) == 4 and p[0] == "audio-clock"
                         and abs(float(p[1]) - marker) < 1e-6, timeout)

    def frame(self, target, timeout=3):
        for event in list(self.frames):
            if abs(float(event[1][2]) - target) < 1e-6:
                return event
        return self.wait(lambda p: len(p) == 5 and p[0] == "video-frame"
                         and abs(float(p[2]) - target) < 1e-6, timeout)

    def close(self):
        try:
            if self.process.poll() is None:
                self.send("quit")
                self.process.stdin.close()
                self.process.wait(timeout=3)
        except (OSError, subprocess.TimeoutExpired):
            self.process.kill()
            self.process.wait()
        finally:
            for reader in self.readers:
                reader.join(timeout=1)
            for stream in (self.process.stdin, self.process.stdout, self.process.stderr):
                stream.close()


@unittest.skipUnless(os.name == "nt", "Native pointer bridge requires Windows")
class PreviewPointerTests(unittest.TestCase):
    def setUp(self):
        self.host = Host()
        self.addCleanup(self.host.close)
        self.user = ctypes.WinDLL("user32", use_last_error=True)
        # Match the host's physical-pixel context. An unaware SendMessage caller
        # otherwise gets its synthetic mouse coordinates DPI-virtualized.
        self.user.SetThreadDpiAwarenessContext.argtypes = [ctypes.c_void_p]
        self.user.SetThreadDpiAwarenessContext.restype = ctypes.c_void_p
        previous_dpi = self.user.SetThreadDpiAwarenessContext(-4)  # PER_MONITOR_AWARE_V2
        self.assertTrue(previous_dpi)
        self.addCleanup(self.user.SetThreadDpiAwarenessContext, previous_dpi)
        self.user.SendMessageW.argtypes = [wintypes.HWND, wintypes.UINT,
                                          wintypes.WPARAM, wintypes.LPARAM]
        self.user.SendMessageW.restype = ctypes.c_ssize_t
        self.user.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
        self.user.GetClassNameW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
        self.host.send("size-screen 10 10 640 360 0\ntransport 15000 0")
        self.host.clock(15000)
        windows = []
        callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)

        def collect(hwnd, _):
            process = wintypes.DWORD()
            self.user.GetWindowThreadProcessId(hwnd, ctypes.byref(process))
            name = ctypes.create_unicode_buffer(256)
            self.user.GetClassNameW(hwnd, name, len(name))
            if process.value == self.host.process.pid and name.value == "TodoPreviewHost":
                windows.append(hwnd)
            return True

        callback = callback_type(collect)
        self.user.EnumWindows.argtypes = [callback_type, wintypes.LPARAM]
        self.user.EnumWindows(callback, 0)
        self.assertEqual(len(windows), 1, "test host did not create its hidden preview popup")
        self.hwnd = windows[0]

    def message(self, message, x=0, y=0, buttons=0):
        position = (x & 0xffff) | ((y & 0xffff) << 16)
        return self.user.SendMessageW(self.hwnd, message, buttons, position)

    def pointer(self, kind):
        return self.host.wait(lambda p: len(p) == 4 and p[:2] == ["preview-pointer", kind], timeout=0.5)[1]

    def navigation(self, kind):
        return self.host.wait(lambda p: len(p) == 5 and p[:2] == ["preview-navigation", kind],
                              timeout=0.5)[1]

    def test_middle_drag_reports_navigation_and_finishes_after_capture_loss(self):
        self.message(0x0207, 320, 90, 16)  # WM_MBUTTONDOWN
        self.assertEqual(self.navigation("pan-down")[3:], ["0.500000000", "0.250000000"])
        self.message(0x0200, -160, 540, 16)
        self.assertEqual(self.navigation("pan-move")[3:], ["-0.250000000", "1.500000000"])
        self.message(0x001F)  # WM_CANCELMODE
        self.assertEqual(self.navigation("pan-up")[3:], ["-0.250000000", "1.500000000"])
        self.message(0x0208, 0, 0)
        with self.assertRaisesRegex(AssertionError, "timed out"):
            self.navigation("pan-up")

    def test_wheel_uses_window_fractions_and_signed_wheel_delta(self):
        # Wheel coordinates are screen pixels; the hidden popup starts at (10, 10).
        self.message(0x020A, 330, 100, 120 << 16)
        self.assertEqual(self.navigation("wheel")[2:], ["1.000000000", "0.500000000", "0.250000000"])
        self.message(0x020A, 330, 100, (-120 & 0xffff) << 16)
        self.assertEqual(self.navigation("wheel")[2], "-1.000000000")

    def test_drag_reports_canvas_fractions_outside_capture_bounds(self):
        self.message(0x0201, 320, 90, 1)  # WM_LBUTTONDOWN
        self.assertEqual(self.pointer("down")[2:], ["0.500000000", "0.250000000"])
        self.message(0x0200, -160, 540, 1)  # WM_MOUSEMOVE
        self.assertEqual(self.pointer("move")[2:], ["-0.250000000", "1.500000000"])
        self.message(0x0202, 720, -90)  # WM_LBUTTONUP
        self.assertEqual(self.pointer("up")[2:], ["1.125000000", "-0.250000000"])

    def test_lost_capture_finishes_once_at_last_pointer_position(self):
        self.message(0x0201, 160, 180, 1)
        self.pointer("down")
        self.user.SendMessageW(self.hwnd, 0x0215, 0, 0)  # WM_CAPTURECHANGED
        self.assertEqual(self.pointer("up")[2:], ["0.250000000", "0.500000000"])
        self.message(0x0200, 320, 180, 1)
        self.message(0x0202, 320, 180)
        with self.assertRaisesRegex(AssertionError, "timed out"):
            self.host.wait(lambda p: p and p[0] == "preview-pointer", timeout=0.1)

    def test_popup_keeps_noactivate_mouse_behavior(self):
        result = self.user.SendMessageW(self.hwnd, 0x0021, 0, 0)  # WM_MOUSEACTIVATE
        self.assertEqual(result, 3)  # MA_NOACTIVATE


@unittest.skipUnless(os.name == "nt", "Media Foundation preview requires Windows")
class PreviewTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        artifacts = ROOT / "target/preview_tests"
        artifacts.mkdir(parents=True, exist_ok=True)
        cls.temp = tempfile.TemporaryDirectory(prefix="run-", dir=artifacts)
        cls.addClassCleanup(cls.temp.cleanup)
        ffmpeg = ROOT / "dependencies/ffmpeg/bin/ffmpeg.exe"
        if not ffmpeg.is_file():
            ffmpeg = shutil.which("ffmpeg")
        if not ffmpeg:
            raise RuntimeError("FFmpeg is required to generate video test fixtures")
        supplied = os.environ.get("TODO_TEST_MEDIA")
        cls.media = Path(supplied).resolve() if supplied else Path(cls.temp.name) / "long-gop.mp4"
        if not supplied:
            cls._generate(ffmpeg, cls.media, "testsrc2=size=1920x1080:rate=30", "10")
        if not cls.media.is_file():
            raise RuntimeError(f"Missing test video: {cls.media}")
        cls.short = Path(cls.temp.name) / "short video.mp4"
        cls._generate(ffmpeg, cls.short, "color=c=blue:size=320x180:rate=30", "1")
        cls.anamorphic = Path(cls.temp.name) / "anamorphic SAR 16-15.mp4"
        cls._generate(ffmpeg, cls.anamorphic,
                      "color=c=red:size=720x576:rate=25,setsar=16/15", "1")

    @staticmethod
    def _generate(ffmpeg, path, source, duration):
        result = subprocess.run(
            [str(ffmpeg), "-v", "error", "-f", "lavfi", "-i", source,
             "-t", duration, "-c:v", "libx264", "-preset", "ultrafast",
             "-g", "250", "-keyint_min", "250", "-sc_threshold", "0",
             "-pix_fmt", "yuv420p", str(path)],
            capture_output=True, timeout=60, creationflags=NO_WINDOW,
        )
        if result.returncode:
            raise RuntimeError(result.stderr.decode("utf-8", "replace"))

    def setUp(self):
        self.host = Host()
        self.addCleanup(self.host.close)
        self.host.send(f"path {self.media}\nseek 8\ntransport 10000 0")
        self.host.clock(10000, timeout=10)
        self.host.frame(8, timeout=10)

    def reverse_scrub(self):
        start = time.perf_counter()
        for i in range(24):
            target = 7.8 - i * 0.1
            self.host.send(f"transport {target:.6f} 0\nseek {target:.6f}")
            delay = start + (i + 1) / 60 - time.perf_counter()
            if delay > 0:
                time.sleep(delay)
        return time.perf_counter()

    def assert_contains(self, parts, target):
        stamp, duration = float(parts[3]), float(parts[4])
        self.assertLessEqual(stamp, target + 1e-5)
        self.assertGreater(stamp + duration, target - 1e-5)

    def test_control_stays_responsive_while_reverse_scrubbing(self):
        stopped = self.reverse_scrub()
        self.host.send("transport 10001 0")
        stamp, _ = self.host.clock(10001, timeout=1)
        latency = stamp - stopped
        print(f"\ncontrol response after reverse scrub: {latency * 1000:.1f} ms", flush=True)
        self.assertLess(latency, 0.8, "Video decoding blocked transport commands")

    def test_final_reverse_target_is_decoded(self):
        stopped = self.reverse_scrub()
        stamp, parts = self.host.frame(5.5, timeout=2)
        self.assert_contains(parts, 5.5)
        print(f"\nfinal frame after reverse scrub: {(stamp - stopped) * 1000:.1f} ms", flush=True)
        self.assertFalse(self.host.errors)

    def test_reverse_drag_updates_before_release(self):
        start = time.perf_counter()
        for i in range(60):
            target = 7.8 - i * 0.1
            self.host.send(f"transport {target:.6f} 0\nseek {target:.6f}")
            delay = start + (i + 1) / 60 - time.perf_counter()
            if delay > 0:
                time.sleep(delay)
        stopped = time.perf_counter()
        self.host.frame(1.9)
        updates = [p for stamp, p in list(self.host.frames)
                   if start <= stamp < stopped and float(p[2]) < 7.9]
        print(f"\nreverse updates during drag: {len(updates)}/60", flush=True)
        self.assertGreaterEqual(len(updates), 20, "Continuous reverse input rarely updated preview")

    def test_source_change_replaces_pending_seek(self):
        self.reverse_scrub()
        self.host.send(f"path {self.short}\nseek 20")
        _, parts = self.host.frame(20)
        self.assertGreaterEqual(float(parts[3]), 0.9)
        self.assertLess(float(parts[3]), 1.0, "Frame came from the previous source")
        self.assertFalse(self.host.errors)

    def test_cached_reverse_then_forward_and_eof_are_precise(self):
        # Move between cached preroll frames, cross the reader's real cursor,
        # reach EOF, then reverse and resume beyond the cached display again.
        for target in (7.9, 7.7, 7.85, 8.2, 7.6, 8.7):
            self.host.send(f"seek {target}")
            _, parts = self.host.frame(target)
            self.assert_contains(parts, target)
        self.host.send(f"path {self.short}\nseek 20")
        _, eof = self.host.frame(20)
        self.assertGreaterEqual(float(eof[3]), 0.9)
        self.assertLess(float(eof[3]), 1)
        for target in (0.7, 0.9, 0.95):
            self.host.send(f"seek {target}")
            _, parts = self.host.frame(target)
            self.assert_contains(parts, target)
        self.assertFalse(self.host.errors)

    def test_submillisecond_seek_across_frame_boundary_is_precise(self):
        # Both times round to 33 ms, but lie on opposite sides of 1/30 s.
        self.host.send(f"path {self.short}\nseek 0.0332")
        _, before = self.host.frame(0.0332)
        self.assert_contains(before, 0.0332)
        self.host.send("seek 0.0334")
        _, after = self.host.frame(0.0334)
        self.assert_contains(after, 0.0334)
        self.assertGreater(float(after[3]), float(before[3]))
        self.assertFalse(self.host.errors)

    def test_reopening_source_discards_cache(self):
        self.host.send(f"path {self.short}\nseek 0.5")
        _, short = self.host.frame(0.5)
        self.assert_contains(short, 0.5)
        self.host.frames.clear()
        self.host.send(f"path {self.media}\nseek 0.5")
        _, original = self.host.frame(0.5)
        self.assert_contains(original, 0.5)
        self.assertFalse(self.host.errors)

    def test_hidden_preview_survives_successive_frame_handoffs(self):
        # Exercise D3D presentation and decoder-surface leases without showing UI.
        self.host.send("size-screen 0 0 640 360 0")
        for target in (8.1, 8.2, 8.3, 8.4, 8.5, 8.6, 8.45, 8.55):
            self.host.send(f"seek {target}")
            _, parts = self.host.frame(target)
            self.assert_contains(parts, target)
        self.assertFalse(self.host.errors)

    def test_transform_redraws_retained_frame_without_decoding(self):
        self.host.send("size-screen 0 0 640 360 0")
        original_frames = len(self.host.frames)
        for transform in ("0 0 1", "0.25 -0.5 2", "-2 3 0.1", "0 0 1 0.5 2 0.25",
                          "0 0 1 1 1 0", "0 0 0", "NaN 0 1", "0 0 1 extra",
                          "0 0 1 0 1 1", "0 0 1 1 1 1.5"):
            self.host.send(f"transform {transform}")
        for gizmo in ("-0.25 0.1 1.25 0.9 1", "0 0 1 1 0", "0 0 0 1 1", "nan 0 1 1 1", ""):
            self.host.send(f"gizmo {gizmo}".strip())
        for guides in ("0 0.5 1 0.25", "2 0.5", "0 nan", ""):
            self.host.send(f"guides {guides}".strip())
        self.host.send("transport 10003 0")
        self.host.clock(10003, timeout=1)
        self.assertEqual(len(self.host.frames), original_frames,
                         "Transform unexpectedly triggered video decoding")
        self.assertFalse(self.host.errors)

    def test_source_aspect_uses_decoded_sar_and_source_generation(self):
        def aspect(generation):
            return self.host.wait(lambda p: len(p) == 3 and p[0] == "source-aspect"
                                  and int(p[1]) == generation, timeout=3)[1]

        self.host.send(f"path-token 11 {self.anamorphic}\nseek 0.25")
        self.assertAlmostEqual(float(aspect(11)[2]), 4 / 3, places=6)
        self.host.frame(0.25)
        original_frames = len(self.host.frames)
        self.host.send("transform 0 0 1 2 0.5 0\ngizmo 0 0 1 1 1\ntransport 10005 0")
        self.host.clock(10005)
        self.assertEqual(len(self.host.frames), original_frames)
        self.host.send("seek 0.5\ntransport 10006 0")
        self.host.clock(10006)
        self.host.frame(0.5)
        with self.assertRaisesRegex(AssertionError, "timed out"):
            self.host.wait(lambda p: p and p[0] == "source-aspect" and p[1] == "11", timeout=0.1)

        self.host.send(f"path-token 12 {self.anamorphic}")
        self.assertAlmostEqual(float(aspect(12)[2]), 4 / 3, places=6)
        self.host.send("path-token 13\ntransport 10007 0")
        self.host.clock(10007)
        with self.assertRaisesRegex(AssertionError, "timed out"):
            self.host.wait(lambda p: p and p[0] == "source-aspect" and p[1] == "13", timeout=0.1)
        self.host.send(f"path-token 14 {self.short}")
        self.assertAlmostEqual(float(aspect(14)[2]), 16 / 9, places=6)
        self.host.send(f"path-token 15 {self.media}\nseek 8\npath-token 16 {self.anamorphic}\nseek 0.75")
        self.assertAlmostEqual(float(aspect(16)[2]), 4 / 3, places=6)
        self.host.send(f"path {self.short}")
        self.assertAlmostEqual(float(aspect(0)[2]), 16 / 9, places=6)
        self.assertFalse(self.host.errors)

    def test_clearing_source_discards_pending_frames(self):
        self.reverse_scrub()
        self.host.send("path\ntransport 10002 0")
        self.host.clock(10002, timeout=1)
        with self.assertRaisesRegex(AssertionError, "timed out"):
            self.host.wait(lambda p: p and p[0] == "video-frame", timeout=0.4)

    def test_quit_interrupts_obsolete_decoding(self):
        self.reverse_scrub()
        self.host.send("quit")
        self.host.process.wait(timeout=1.5)
        self.assertEqual(self.host.process.returncode, 0)


if __name__ == "__main__":
    unittest.main()
