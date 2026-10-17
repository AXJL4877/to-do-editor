import importlib.util
import tempfile
import unittest
from pathlib import Path

import todo_worker


todo_worker.Worker.serve = lambda self: None
MODULE = Path(__file__).parents[1] / "worker" / "main.py"
SPEC = importlib.util.spec_from_file_location("spongebob_worker", MODULE)
WORKER = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(WORKER)


class Context:
    canvas = {"fps": 30}
    params = {}

    def __init__(self, root):
        self.workdir = root
        self.calls = []

    def path(self, relative):
        path = self.workdir / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        return path

    def call(self, service, request):
        self.calls.append((service, request))
        return {"file": "services/wipe-test.mp4", "duration": 0.3}


class WorkerUnitTests(unittest.TestCase):
    def test_bgm_plan_uses_valid_chat_times_and_keeps_safe_defaults(self):
        plan = WORKER._design_bgm({"text": {"bgm_plan": {
            "intro": {"at": 0.4},
            "mid": {"start": 2.0},
            "outro": {"at": "bad"},
        }}}, 8.0)
        self.assertEqual(plan, {"intro": 0.4, "mid": 2.0, "outro": 6.0})

    def test_subtitle_cues_become_editable_text_items(self):
        items = WORKER._subtitle_items(2, 1.5, {
            "cues": [
                {"text": "第一句", "start": 0.2, "end": 0.8},
                {"text": "第二句", "start": 0.9, "end": 1.4},
            ]
        })
        self.assertEqual([item["key"] for item in items], ["sub/2/0", "sub/2/1"])
        self.assertEqual(items[0]["source"]["type"], "text")
        self.assertEqual(items[0]["source"]["content"]["text"], "第一句")
        self.assertAlmostEqual(items[1]["start"], 2.4)
        self.assertAlmostEqual(items[1]["duration"], 0.5)

    def test_online_wipe_uses_host_packed_alpha_encode(self):
        with tempfile.TemporaryDirectory() as directory:
            ctx = Context(Path(directory))
            items = WORKER._transition_items(ctx, 1, 1.0, 1.0, "转场", encode=True)
            self.assertEqual(len(items), 2)
            self.assertEqual(items[0]["source"]["file"], "services/wipe-test.mp4")
            self.assertEqual(ctx.calls[0][0], "encode")
            self.assertTrue(ctx.calls[0][1]["alpha"])
            self.assertTrue(list((Path(directory) / "out/wipe-01.frames").glob("*.png")))
            self.assertTrue((Path(directory) / "out/wipe-01.wav").is_file())


if __name__ == "__main__":
    unittest.main()
