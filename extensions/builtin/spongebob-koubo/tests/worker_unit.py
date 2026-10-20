"""Offline checks of the recipe worker (v5): shots, labels, motions, items.

    dependencies/python/python.exe extensions/builtin/spongebob-koubo/tests/worker_unit.py
"""
import importlib.util
import json
import sys
import tempfile
import unittest

from PIL import Image
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "extensions" / "sdk" / "python"))
MODULE = Path(__file__).parents[1] / "worker" / "main.py"
SPEC = importlib.util.spec_from_file_location("spongebob_worker", MODULE)
W = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(W)
FIXTURE = json.loads((Path(__file__).parent / "fixture.json").read_text(encoding="utf-8"))


class Ctx(W.OfflineContext):
    """The worker's own offline context, with a call log, emit batches and
    optional failing pictures."""

    def __init__(self, root, plan=None, fail=(), params=None):
        class Base:
            pass
        base = Base()
        base.workdir = Path(root)
        base.params = {"topic": "沉没成本", "seconds": "30", **(params or {})}
        base.input = {"script": FIXTURE["script"]}
        base.canvas = {"width": 1920, "height": 1080, "fps": 30}
        base.seed = 0
        base.batches, base.calls = [], []

        def path(relative):
            p = base.workdir / relative
            p.parent.mkdir(parents=True, exist_ok=True)
            return p
        base.path = path
        base.progress = lambda *_: None
        base.emit = lambda items: base.batches.append(list(items))
        super().__init__(base)
        self._fail = fail
        if plan is not None:
            self._fixture = {**self._fixture, "plan": plan}

    def call(self, service, request):
        self._ctx.calls.append((service, request))
        if service == "image" and any(word in request["prompt"] for word in self._fail):
            raise W.ServiceError("ambiguous", "no complete image")
        return super().call(service, request)

    @property
    def items(self):
        return {i["key"]: i for batch in self._ctx.batches for i in batch}


def run(root, **kwargs):
    ctx = Ctx(root, **kwargs)
    return W.run(ctx), ctx


class ShotTest(unittest.TestCase):
    def test_one_sentence_one_shot_short_ones_merge(self):
        sentences = [{"text": "第一句很长很长。", "start": 0.0, "end": 2.0},
                     {"text": "短。", "start": 2.2, "end": 2.8},
                     {"text": "第三句。", "start": 3.0, "end": 5.0}]
        shots = W.split_shots(sentences, [])
        self.assertEqual([s["text"] for s in shots], ["第一句很长很长。短。", "第三句。"])

    def test_long_sentences_split_at_a_subtitle_break(self):
        sentences = [{"text": "一二三四五六七八九十，一二三四五六七八九十。", "start": 0.0, "end": 8.0}]
        cues = [{"text": "一二三四五六七八九十", "start": 0.0, "end": 3.9},
                {"text": "一二三四五六七八九十", "start": 4.1, "end": 8.0}]
        shots = W.split_shots(sentences, cues)
        self.assertEqual([(s["start"], s["end"]) for s in shots], [(0.0, 3.9), (4.1, 8.0)])

    def test_word_times_come_from_the_characters(self):
        chars = FIXTURE["speech"]["timeline"]["chars"]
        shot = {"start": 0.0, "end": 3.816, "text": "蟹老板花五十贝壳买了电影票。"}
        t = W.word_time(chars, shot, "电影票")
        self.assertAlmostEqual(t, chars[10]["start"])
        self.assertIsNone(W.word_time(chars, shot, "不存在"))


class LineTest(unittest.TestCase):
    def test_long_yellow_lines_break_at_a_stop_or_comma(self):
        self.assertEqual(W._spoken_line("别让第一眼价格替你拿主意。你看清那个锚了吗？", 12),
                         "别让第一眼价格替你拿主意\n你看清那个锚了吗？")
        # A comma far from the middle would leave a stub line: break midway.
        self.assertEqual(W._spoken_line("下次买东西先问自己原本想花多少，再决定。", 12),
                         "下次买东西先问自己\n原本想花多少，再决定")
        self.assertEqual(W._spoken_line("看清诱饵守住理性。", 12), "看清诱饵守住理性")


class LabelTest(unittest.TestCase):
    def setUp(self):
        W.LIBRARY = self.library = W._library()
        sentences = FIXTURE["speech"]["timeline"]["sentences"]
        self.shots = W.split_shots(sentences, FIXTURE["cues"])

    def test_answers_are_checked_per_shot(self):
        plan, problems = W.check_labels(json.loads(json.dumps(FIXTURE["plan"])), self.shots, self.library)
        shots = plan["shots"]
        self.assertEqual(len(shots), 7)
        self.assertIsNone(shots[2]["motion"], "unknown motion becomes random")
        self.assertTrue(any("spin" in p for p in problems))
        # The card's own character is not repeated as an element.
        self.assertEqual([e.get("lib") for e in shots[2]["elements"]], ["mark-question"])
        self.assertEqual(shots[1]["elements"][1]["texts"], ["后悔"])

    def test_sublime_is_capped_at_a_fifth_keeping_the_end(self):
        plan, problems = W.check_labels(json.loads(json.dumps(FIXTURE["plan"])), self.shots, self.library)
        kinds = [s["kind"] for s in plan["shots"]]
        self.assertEqual(kinds[-2:], ["story", "sublime"])
        self.assertEqual(plan["shots"][5]["elements"][0]["lib"], "money-bag", "demoted shot keeps its elements")
        self.assertTrue(any("20%" in p for p in problems))

    def test_a_line_about_a_character_is_never_an_uplift(self):
        answer = json.loads(json.dumps(FIXTURE["plan"]))
        answer["shots"][0]["kind"] = "sublime"  # “蟹老板花五十贝壳买了电影票”
        plan, problems = W.check_labels(answer, self.shots, self.library)
        self.assertEqual(plan["shots"][0]["kind"], "story")
        self.assertTrue(any("蟹老板" in p for p in problems), problems)
        self.assertEqual(plan["shots"][-1]["kind"], "sublime", "a plain lesson stays")

    def test_a_missing_answer_falls_back_per_shot(self):
        plan, problems = W.check_labels({"shots": [{"i": 0, "kind": "story", "draw": "x"}]},
                                        self.shots, self.library)
        self.assertEqual(plan["shots"][0]["draw"], "x")
        self.assertTrue(all(s["kind"] == "story" for s in plan["shots"]))
        self.assertIn("沉没成本", plan["shots"][4]["draw"])
        self.assertEqual(len(problems), 6)

    def test_motions_are_seeded_and_never_repeat_back_to_back(self):
        import random
        picks = []
        for _ in range(2):
            plan, _ = W.check_labels({}, self.shots * 3, self.library)
            W.choose_motions(plan, random.Random("same"))
            picks.append([s["motion"] for s in plan["shots"]])
        self.assertEqual(picks[0], picks[1])
        self.assertEqual(picks[0][0], "grow")
        self.assertTrue(all(a != b for a, b in zip(picks[0], picks[0][1:])))


def shape(width, height, cx=0.5, ink=0.6):
    return {"size": (width, height), "cx": cx, "ink": ink}


def element(w, h, relation="beside", size="small", **extra):
    return {"shape": shape(w, h), "relation": relation, "size": size, **extra}


def edges(box):
    return box["x"] - box["width"] / 2, box["x"] + box["width"] / 2


class LayoutTest(unittest.TestCase):
    MARGIN = W.RECIPE["layout"]["margin"]

    def assert_inside(self, placed):
        for box in [placed["card"]] + placed["elements"]:
            left, right = edges(box)
            self.assertGreaterEqual(left, -0.5 + self.MARGIN - 1e-6, box)
            self.assertLessEqual(right, 0.5 - self.MARGIN + 1e-6, box)
            self.assertGreaterEqual(box["bottom"] - box["height"], W.RECIPE["layout"]["top"] - 1e-6)

    def test_the_screenshot_case_stays_compact_and_centred(self):
        # Two fish (wide card), a newspaper and a burger: the burger used to
        # sit in the bottom-right corner with 2 % margin.
        card = {"shape": shape(1400, 1000)}
        placed = W.compose(card, [element(900, 800, size="medium"), element(800, 640)], "left")
        self.assert_inside(placed)
        boxes = [placed["card"]] + placed["elements"]
        left = min(edges(b)[0] for b in boxes)
        right = max(edges(b)[1] for b in boxes)
        self.assertLess(abs(left + right), 0.08, "the group is about centred")
        # Even gaps along the ground row.
        row = sorted(boxes, key=lambda b: b["x"])
        gaps = [edges(b)[0] - edges(a)[1] for a, b in zip(row, row[1:])]
        self.assertAlmostEqual(gaps[0], gaps[1], places=3)
        # A small prop is smaller than a character on the card.
        burger = placed["elements"][1]
        self.assertLess(burger["height"], 0.25)
        self.assertTrue(all(b["bottom"] == W.RECIPE["layout"]["ground"] for b in boxes))

    def test_symbols_float_above_and_stack_on_their_target(self):
        card = {"shape": shape(1000, 1000)}
        sign = element(700, 1000, size="medium", side="right")
        cross = element(1000, 1000, relation="on", target=0)
        question = element(600, 1000, relation="above")
        placed = W.compose(card, [sign, cross, question], "left")
        self.assert_inside(placed)
        sign_box, cross_box, question_box = placed["elements"]
        self.assertAlmostEqual(cross_box["x"], sign_box["x"], places=4)
        self.assertLess(cross_box["height"], sign_box["height"])
        card_top = placed["card"]["bottom"] - placed["card"]["height"]
        self.assertLessEqual(question_box["bottom"], card_top)

    def test_open_sides_follow_the_shot_mirror_and_given_sides_are_kept(self):
        card = {"shape": shape(1000, 1000)}
        lefty = W.compose(card, [element(800, 800)], "right")
        righty = W.compose(card, [element(800, 800)], "left")
        self.assertLess(lefty["elements"][0]["x"], lefty["card"]["x"])
        self.assertGreater(righty["elements"][0]["x"], righty["card"]["x"])
        fixed = W.compose(card, [element(800, 800, side="right")], "right")
        self.assertGreater(fixed["elements"][0]["x"], fixed["card"]["x"])

    def test_a_crowded_shot_shrinks_instead_of_leaving_the_frame(self):
        card = {"shape": shape(1600, 900)}
        placed = W.compose(card, [element(1400, 900, size="large"), element(1400, 900, size="large")],
                           "left")
        self.assert_inside(placed)

    def test_floats_on_one_target_stand_side_by_side(self):
        card = {"shape": shape(1000, 1000)}
        placed = W.compose(card, [element(600, 1000, relation="above"),
                                  element(800, 1000, relation="above")], "left")
        self.assert_inside(placed)
        first, second = placed["elements"]
        self.assertEqual(W._overlap(first, second), 0.0)
        card_top = placed["card"]["bottom"] - placed["card"]["height"]
        self.assertTrue(all(b["bottom"] <= card_top for b in (first, second)))

    def test_a_float_without_room_overhead_hangs_beside_not_on_the_target(self):
        tall = {"shape": shape(400, 1000)}
        placed = W.compose(tall, [element(600, 1000, relation="above", size="medium")], "left")
        mark = placed["elements"][0]
        self.assertEqual(W._overlap(mark, placed["card"]), 0.0)
        self.assert_inside(placed)

    def test_the_word_front_now_means_beside(self):
        self.assertEqual(W._check_place({"relation": "front"}), {"relation": "beside"})


class CalmTest(unittest.TestCase):
    def test_short_shots_get_fewer_elements(self):
        limits = W.RECIPE["elements"]["minShot"]
        def plan_for(seconds):
            shot = {"kind": "story", "audio": {"start": 0.0, "end": seconds, "text": "x"},
                    "elements": [{"lib": "a", "at": ""}, {"lib": "b", "at": ""}]}
            span = (0.0, seconds)
            W.calm_elements({"shots": [shot]}, [span], {0: span}, 0.0, [], {0: {"enter": 0.5}})
            return len(shot["elements"])
        self.assertEqual(plan_for(limits["1"] - 0.1), 0)
        self.assertEqual(plan_for(limits["2"] - 0.1), 1)
        self.assertEqual(plan_for(limits["2"] + 1.5), 2)

    def test_elements_keep_a_calm_beat_and_stay_long_enough(self):
        limits = W.RECIPE["elements"]
        shot = {"audio": {"start": 0.0, "end": 5.0, "text": "x"},
                "elements": [{"at": ""}, {"at": ""}, {"at": ""}]}
        times = W.element_times(shot, (1.4, 6.0), 1.4, [], 0.6)
        kept = [t for t in times if t is not None]
        self.assertGreaterEqual(kept[0], 1.4 + limits["lead"])
        for a, b in zip(kept, kept[1:]):
            self.assertGreaterEqual(b - a, limits["stagger"] - 1e-6)
        for t in kept:
            self.assertLessEqual(t, 6.0 - limits["minShow"] + 1e-6)

    def test_dropping_an_element_moves_targets_with_it(self):
        shot = {"elements": [{"lib": "a"}, {"lib": "b"}, {"lib": "c", "relation": "on", "target": 1}]}
        W._keep_elements(shot, [1, 2])
        self.assertEqual(shot["elements"][1]["target"], 0)
        W._keep_elements(shot, [1])
        self.assertIsNone(shot["elements"][0]["target"])


class RunTest(unittest.TestCase):
    def test_fast_parts_are_placed_before_any_picture(self):
        with tempfile.TemporaryDirectory() as root:
            _, ctx = run(root)
            first = {i["key"] for i in ctx._ctx.batches[0]}
            self.assertEqual(first, {"voice", "bgm/0", "bgm/1", "title/card", "title/text"})
            # Each story shot arrives as one batch (card and its elements,
            # laid out together), after the plan's words.
            shot_batches = [b for b in ctx._ctx.batches
                            if any(i["key"].startswith("shot/") and i["key"].endswith("/card") for i in b)]
            self.assertEqual(len(shot_batches), 6, "six story shots (one sublime was capped)")
            for batch in shot_batches:
                shots = {i["key"].split("/")[1] for i in batch}
                self.assertEqual(len(shots), 1, "one shot per batch")

    def test_story_shots_have_a_card_and_timed_elements(self):
        with tempfile.TemporaryDirectory() as root:
            result, ctx = run(root)
            items = ctx.items
            self.assertEqual(result["shots"], 7)
            card0, card1 = items["shot/0/card"], items["shot/1/card"]
            self.assertEqual(card0["intro"]["kind"], "sb.growIn")
            self.assertEqual(card1["transition"]["kind"], "sb.cardDrop")
            # A lone card is centred; with an element beside it, it moves aside.
            self.assertAlmostEqual(items["shot/3/card"]["transform"]["x"], 0.0, places=2)
            self.assertNotAlmostEqual(card0["transform"]["x"], 0.0, places=2)
            # An element pops in when its words are spoken.
            chars = FIXTURE["speech"]["timeline"]["chars"]
            coins = items["shot/0/el/0"]
            # When its words are spoken, but only once the card has landed.
            landed = card0["start"] + card0["intro"]["duration"] + 0.2
            self.assertAlmostEqual(coins["start"], max(1.4 + chars[4]["start"], landed), places=2)
            self.assertEqual(coins["intro"]["kind"].split(".")[0], "sb")
            # The sign's words come late in its line: it comes in a little
            # earlier, a calm beat after the clock, and stays long enough.
            clock, sign = items["shot/1/el/0"], items["shot/1/el/1"]
            limits = W.RECIPE["elements"]
            self.assertGreaterEqual(sign["start"] - clock["start"], limits["stagger"] - 1e-3)
            self.assertGreaterEqual(sign["duration"], limits["minShow"] - 1e-3)
            self.assertEqual(sign["intro"]["kind"], "sb.elPlop")
            self.assertEqual(W.SYMBOL_MOTION["mark-x"], "stamp", "a red cross is stamped on")
            # Floating symbols hang higher than things on the ground.
            self.assertLess(items["shot/2/el/0"]["transform"]["y"], items["shot/1/el/0"]["transform"]["y"])
            # Elements are gone before the next card starts crossing in (the
            # window is centred on the cut).
            for key, item in items.items():
                if "/el/" in key:
                    end = item["start"] + item["duration"]
                    k = int(key.split("/")[1])
                    card = items[f"shot/{k}/card"]
                    cut = card["start"] + card["duration"]
                    following = items.get(f"shot/{k + 1}/card", {}).get("transition")
                    leave = cut - (following["duration"] / 2 if following else 0)
                    self.assertAlmostEqual(end, leave, places=2, msg=key)

    def test_sublime_is_a_blurred_picture_with_the_red_line_and_no_subtitle(self):
        with tempfile.TemporaryDirectory() as root:
            result, ctx = run(root)
            items = ctx.items
            line = items["shot/6/line"]
            content = line["source"]["content"]
            self.assertEqual(content["styleId"], "sublime-red-brush")
            self.assertEqual(content["text"], "别让它决定你的下一步")
            self.assertEqual(content["animation"]["intro"]["kind"], "perChar")
            # The backdrop: the shot's picture, blurred on black, over the
            # black ground and under the line, pushing in for the whole shot.
            backdrop = items["shot/6/backdrop"]
            self.assertTrue(backdrop["source"]["file"].endswith("-blurred.jpg"))
            self.assertEqual(backdrop["intro"]["kind"], "sb.backdropZoom")
            self.assertAlmostEqual(backdrop["intro"]["duration"], backdrop["duration"], places=2)
            with Image.open(Path(root) / backdrop["source"]["file"]) as picture:
                self.assertEqual(picture.size, (1920, 1080))
                self.assertEqual(picture.getpixel((5, 5)), (0, 0, 0), "black at the edges")
            prompts = [r["prompt"] for s, r in ctx._ctx.calls if s == "image"]
            self.assertTrue(any("looking forward at a road" in p and "No text" in p for p in prompts))
            grounds = sorted((i for k, i in items.items() if k.startswith("bg/")), key=lambda i: i["start"])
            self.assertTrue(grounds[-1]["source"]["file"].endswith("black.png"))
            self.assertLessEqual(grounds[-1]["start"], line["start"])
            subs = [i for k, i in items.items() if k.startswith("sub/")]
            self.assertEqual(len(subs), 6)
            for sub in subs:
                self.assertLessEqual(sub["start"] + sub["duration"], line["start"] + 0.2)
            self.assertNotIn("shot/6/card", items)
            self.assertLessEqual(result["sublimeShare"], 0.2)

    def test_a_card_that_never_arrives_uses_the_character(self):
        with tempfile.TemporaryDirectory() as root:
            result, ctx = run(root, fail=("yawning",))
            card = ctx.items["shot/3/card"]
            self.assertTrue(card["source"]["file"].endswith("krabs-worried.png"))
            self.assertTrue(any("shot/3/card" in p for p in result["planProblems"]))

    def test_redraw_draws_only_the_named_picture_again(self):
        self.assertEqual(W.redraws("shot/1/card:1, shot/1/el/1:2,bg/0:1,shot/x/card:1,junk"),
                         {"shot/1/card": 1, "shot/1/el/1": 2})
        revs = []
        for redraw in ("", "shot/1/card:1"):
            with tempfile.TemporaryDirectory() as root:
                _, ctx = run(root, params={"redraw": redraw})
                revs.append({k: i["rev"] for k, i in ctx.items.items()})
        changed = {k for k in revs[0] if revs[0][k] != revs[1][k]}
        self.assertEqual(changed, {"shot/1/card"})

    def test_an_unusable_answer_still_makes_a_film(self):
        with tempfile.TemporaryDirectory() as root:
            result, ctx = run(root, plan="不是 JSON")
            self.assertTrue(result["planProblems"])
            self.assertEqual(result["shots"], 7)
            self.assertIn("shot/6/card", ctx.items)


if __name__ == "__main__":
    unittest.main()
