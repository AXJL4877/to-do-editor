"""Regression checks against the C++ transform math used by the presenter."""

import os
from pathlib import Path
import subprocess
import unittest


@unittest.skipUnless(os.name == "nt", "Native preview requires Windows")
class VisualTransformTests(unittest.TestCase):
    def test_centered_fit_offsets_zoom_and_invalid_input(self):
        host = Path(os.environ.get("TODO_PREVIEW_HOST",
                    Path(__file__).resolve().parents[1] / "build/bin/preview_host.exe"))
        exe = host.parent / "visual_transform_tests.exe"
        result = subprocess.run([str(exe)], capture_output=True, text=True, timeout=5,
                                creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_production_gpu_shaders_axis_opacity_and_gizmo(self):
        host = Path(os.environ.get("TODO_PREVIEW_HOST",
                    Path(__file__).resolve().parents[1] / "build/bin/preview_host.exe"))
        exe = host.parent / "presenter_shader_tests.exe"
        result = subprocess.run([str(exe)], capture_output=True, text=True, timeout=15,
                                creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
