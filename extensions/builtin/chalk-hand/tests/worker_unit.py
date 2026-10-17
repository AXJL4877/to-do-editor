import importlib.util
import tempfile
import unittest
from pathlib import Path

import numpy as np
import todo_worker


todo_worker.Worker.serve = lambda self: None
MODULE = Path(__file__).parents[1] / "worker" / "bake.py"
SPEC = importlib.util.spec_from_file_location("chalk_bake", MODULE)
BAKE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(BAKE)


class Context:
    def __init__(self, root):
        self.workdir = root
        self.calls = []

    def path(self, relative):
        path = self.workdir / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        return path

    def call(self, service, request):
        self.calls.append((service, request))
        return {"file": "services/fallback-packed.mp4"}


class Capture:
    budget = 3

    def setup(self, crop, budget, tone, samples):
        self.raw_frames = []

    def render_to(self, *_args, **_kwargs):
        self.raw_frames = [
            np.full((4, 4, 3), 80, dtype=np.uint8),
            np.full((4, 4, 3), 160, dtype=np.uint8),
        ]

    def pad(self):
        return None


class WorkerUnitTests(unittest.TestCase):
    def test_failed_reveal_replays_frames_and_calls_alpha_encode(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            ctx = Context(root)
            output = root / "out"
            work = root / "work"
            output.mkdir()
            work.mkdir()
            result = BAKE.packed_fallback(
                ctx, output, "element", Capture(), (0, 0, 4, 4), "chalk", 30, 0.2, work
            )
            self.assertEqual(result, "services/fallback-packed.mp4")
            self.assertEqual(ctx.calls[0][0], "encode")
            self.assertTrue(ctx.calls[0][1]["alpha"])
            frames = list((output / "element.frames").glob("*.png"))
            self.assertEqual(len(frames), 2)


if __name__ == "__main__":
    unittest.main()
