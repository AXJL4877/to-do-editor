"""海绵宝宝口播 worker (recipe v5): a fixed, code-driven pipeline.

The code owns the structure; the language model only fills in pictures:

    script (given, or written from a topic)
      → speech with per-character times
      → voice, background music and the red title go on the timeline at once
      → shots: one sentence = one shot (code: short ones merge, long ones
        split at a subtitle break)
      → the model labels each shot `story` or `sublime` and describes its
        pictures; the code checks every answer, caps sublime at 20 % of the
        film and falls back per shot
      → plate / black lane, subtitles (none over sublime), yellow lines
      → every picture is requested at once; each one goes on the timeline
        the moment it is back (no placeholders)

A story shot is the plate, a main scene card and 0-2 extra elements that pop
in when their words are spoken. A sublime shot is black with the line in
yellow brush. Motions the model did not choose are drawn from a seeded
random generator (same project, same choices).
"""
import hashlib
import json
import random
import shutil
from concurrent.futures import as_completed
from pathlib import Path

from PIL import Image
from todo_worker import RunFailed, ServiceError, Worker

VERSION = "5.0.0"
worker = Worker(version=VERSION)

PACKAGE = Path(__file__).resolve().parent.parent
RECIPE = json.loads((PACKAGE / "recipe.json").read_text(encoding="utf-8"))
LIBRARY_DIR = PACKAGE / "assets" / "library"
CANVAS_ASPECT = 16 / 9
PUNCT = set("，。、；：！？,.;:!?…—\"'“”‘’（）()《》【】 \n")
# Library symbols with a fixed meaning keep their natural motion.
SYMBOL_MOTION = {"mark-x": "stamp"}


# ---------------------------------------------------------------- helpers

def _rev(*parts):
    return hashlib.sha256("|".join(map(str, parts)).encode("utf-8")).hexdigest()[:16]


def _library():
    path = LIBRARY_DIR / "library.json"
    if not path.is_file():
        return {}
    entries = json.loads(path.read_text(encoding="utf-8"))
    return {entry["id"]: entry for entry in entries if (LIBRARY_DIR / entry["file"]).is_file()}


def _prompt(name, **values):
    text = (PACKAGE / "prompts" / name).read_text(encoding="utf-8")
    for key, value in values.items():
        text = text.replace("{{" + key + "}}", str(value))
    return text


def _json_reply(reply):
    text = reply.get("text", "") if isinstance(reply, dict) else str(reply)
    start, end = text.find("{"), text.rfind("}")
    if start < 0 or end <= start:
        raise ValueError("no JSON object in reply")
    return json.loads(text[start:end + 1])


def _relative(ctx, path):
    return Path(path).resolve().relative_to(ctx.workdir.resolve()).as_posix()


def _copy_asset(ctx, entry):
    """Library files are copied into the workdir: items reference workdir files."""
    target = ctx.path(f"out/lib/{entry['file']}")
    source = LIBRARY_DIR / entry["file"]
    if not target.is_file() or target.stat().st_size != source.stat().st_size:
        shutil.copyfile(source, target)
    return _relative(ctx, target)


def _solid(ctx, name, color):
    path = ctx.path(f"out/{name}.png")
    if not path.is_file():
        Image.new("RGB", (192, 108), color).save(path)
    return _relative(ctx, path)


def _fit_scale(size, height):
    """Transform scale that makes a picture `height` of the canvas tall.

    At identity a picture is aspect-fit to the canvas: full height unless it
    is wider than 16:9."""
    w, h = size
    aspect = w / h if h else 1.0
    base = 1.0 if aspect <= CANVAS_ASPECT else CANVAS_ASPECT / aspect
    return height / base


def _motion(pair):
    return {"kind": pair[0], "duration": float(pair[1])} if pair else None


def _item(key, rev, role, lane, start, duration, source, **extra):
    start = round(max(0.0, start), 3)
    item = {
        "key": key,
        "rev": rev,
        "role": role,
        "lane": lane,
        "start": start,
        # Rounded on the same grid as `start`, so start + duration of one item
        # equals the next item's start.
        "duration": round(max(0.05, round(start + duration, 3) - start), 3),
        "source": source,
    }
    item.update({k: v for k, v in extra.items() if v is not None})
    return item


def _text(text, style, size, align="center", letter_spacing=0.0, line_height=1.15,
          animation=None):
    content = {
        "text": text,
        "styleId": style,
        "layout": {"fontSize": size, "letterSpacing": letter_spacing,
                   "lineHeight": line_height, "align": align},
    }
    if animation is not None:
        content["animation"] = {"intro": animation}
    return {"type": "text", "content": content}


def _file(relative):
    return {"type": "file", "file": relative}


def _spoken(text):
    return "".join(c for c in str(text) if c not in PUNCT)


# ---------------------------------------------------------------- script

def speech_cps(rate):
    """Spoken characters per second at a speech rate (percent). Measured on
    the default voice: 4.6 at +16 % (three films, 2026-10-19)."""
    return RECIPE["speech"]["charsPerSecondAtRate0"] * (1 + rate / 100)


def write_script(ctx, topic, seconds, rate):
    cps = speech_cps(rate)
    target = max(8, seconds - RECIPE["timing"]["titleSec"] - RECIPE["timing"]["endSec"])
    low, high = int(target * cps * 0.92), int(target * cps * 1.04)
    messages = [{"role": "user", "content": _prompt(
        "script.md", topic=topic, seconds=int(round(target)), min_chars=low, max_chars=high)}]
    text = ""
    for _ in range(2):
        reply = ctx.call("chat", {"messages": messages, "temperature": 0.8})
        text = (reply.get("text", "") if isinstance(reply, dict) else str(reply)).strip()
        text = "".join(line.strip() for line in text.splitlines() if line.strip())
        count = len(_spoken(text))
        if count <= high * 1.08:
            break
        # Too long for the film: one request to cut it down.
        messages += [{"role": "assistant", "content": text},
                     {"role": "user", "content": f"这段有 {count} 字，太长了。删减到 {low}–{high} 字，"
                                                 "保留故事和结尾，只输出正文。"}]
    if len(text) < 20:
        raise RunFailed("文案生成结果太短", "internal")
    return text


# ---------------------------------------------------------------- shots (code)

def split_shots(sentences, cues):
    """One sentence, one shot. A sentence longer than `maxSec` is split at the
    subtitle break nearest its middle (repeatedly); a shot shorter than
    `minSec` joins its shorter neighbour. Times are audio seconds."""
    rules = RECIPE["shots"]
    shots = []
    for sentence in sentences:
        start, end = float(sentence["start"]), float(sentence["end"])
        inside = [c for c in cues if float(c["start"]) >= start - 0.05 and float(c["end"]) <= end + 0.05]
        pieces = [inside] if len(inside) > 1 else [None]
        while True:
            longest = max(range(len(pieces)), key=lambda i: _piece_len(pieces[i], start, end))
            piece = pieces[longest]
            if piece is None or len(piece) < 2 or _piece_len(piece, start, end) <= rules["maxSec"]:
                break
            middle = (float(piece[0]["start"]) + float(piece[-1]["end"])) / 2
            cut = min(range(1, len(piece)), key=lambda i: abs(float(piece[i]["start"]) - middle))
            pieces[longest:longest + 1] = [piece[:cut], piece[cut:]]
        if len(pieces) == 1:
            shots.append({"start": start, "end": end, "text": str(sentence["text"]).strip()})
        else:
            for piece in pieces:
                shots.append({"start": float(piece[0]["start"]), "end": float(piece[-1]["end"]),
                              "text": "".join(str(c["text"]) for c in piece)})
    merged = True
    while merged and len(shots) > 1:
        merged = False
        for i, shot in enumerate(shots):
            if shot["end"] - shot["start"] >= rules["minSec"]:
                continue
            if i == 0:
                j = 1
            elif i == len(shots) - 1:
                j = i - 1
            else:
                before = shots[i - 1]["end"] - shots[i - 1]["start"]
                after = shots[i + 1]["end"] - shots[i + 1]["start"]
                j = i - 1 if before <= after else i + 1
            a, b = sorted((i, j))
            shots[a:b + 1] = [{"start": shots[a]["start"], "end": shots[b]["end"],
                               "text": shots[a]["text"] + shots[b]["text"]}]
            merged = True
            break
    return shots


def _piece_len(piece, start, end):
    if not piece:
        return end - start
    return float(piece[-1]["end"]) - float(piece[0]["start"])


def shot_spans(shots, offset, voice_end):
    """Timeline spans: cuts midway between shots on a millisecond grid shared
    by neighbours (exact same-lane adjacency)."""
    spans = []
    for k, shot in enumerate(shots):
        start = offset if k == 0 else spans[-1][1]
        if k + 1 < len(shots):
            end = offset + (shot["end"] + shots[k + 1]["start"]) / 2
        else:
            end = offset + voice_end + RECIPE["timing"]["tailSec"]
        spans.append((round(start, 3), round(max(end, start + 0.5), 3)))
    return spans


def word_time(chars, shot, words):
    """Audio time at which `words` start inside the shot, or None."""
    words = _spoken(words)
    if not words or not chars:
        return None
    inside = [c for c in chars if shot["start"] - 0.05 <= float(c["start"]) <= shot["end"] + 0.05
              and _spoken(c.get("text", ""))]
    spoken = "".join(_spoken(c["text"]) for c in inside)
    at = spoken.find(words)
    if at < 0:
        return None
    # Characters may hold more than one letter: map the offset back.
    seen = 0
    for c in inside:
        seen += len(_spoken(c["text"]))
        if seen > at:
            return float(c["start"])
    return None


# ---------------------------------------------------------------- labels (model)

def _vocabulary(table):
    return "\n".join(f"- `{key}` {entry['name']}：{entry['use']}" for key, entry in table.items())


def label_shots(ctx, shots, library):
    """Ask the model for each shot's kind and pictures; every answer is
    checked and a bad shot falls back on its own, never the whole plan."""
    total = sum(s["end"] - s["start"] for s in shots)
    share = RECIPE["sublime"]["maxShare"]
    cast = {k: v for k, v in library.items() if v["kind"] == "cast"}
    props = {k: v for k, v in library.items() if v["kind"] == "prop"}
    lines = "\n".join(f"{i}. [{s['end'] - s['start']:.1f}秒] {s['text']}" for i, s in enumerate(shots))
    prompt = _prompt(
        "shots.md", shots=lines, sublime_share=int(share * 100), sublime_max=f"{total * share:.1f}",
        motion_max=motion_budget(len(shots)),
        pairs=max(1, round(len(shots) * RECIPE["elements"]["pairShare"])),
        cast="\n".join(f"- {k}：{v['name']}（{v['desc']}）" for k, v in cast.items()),
        props="\n".join(f"- {k}：{v['name']}（{v['desc']}）{'（悬浮）' if v.get('float') else ''}"
                        for k, v in props.items()),
        card_motions=_vocabulary(RECIPE["cardMotions"]),
        element_motions=_vocabulary(RECIPE["elementMotions"]))
    problems = []
    try:
        answer = _json_reply(ctx.call("chat", {"messages": [{"role": "user", "content": prompt}],
                                               "json": True, "temperature": 0.5}))
    except (ValueError, ServiceError) as error:
        answer, problems = {}, [f"分镜回答无法使用，全部按默认画面：{error}"]
    plan, more = check_labels(answer, shots, library)
    return plan, problems + more


def check_labels(answer, shots, library):
    """Keep what is valid, repair the rest per shot. Returns the plan
    `{"title", "shots": [...]}` with one entry per shot, and the problems."""
    problems = []
    given = answer.get("shots") if isinstance(answer, dict) else None
    by_index = {}
    for entry in given if isinstance(given, list) else []:
        if isinstance(entry, dict) and isinstance(entry.get("i"), int):
            by_index.setdefault(entry["i"], entry)
    out = []
    for i, shot in enumerate(shots):
        entry = by_index.get(i)
        if entry is None:
            problems.append(f"第 {i} 镜没有回答，用默认画面")
            entry = {}
        out.append(_check_shot(i, entry, shot, library, problems))
    _cap_sublime(out, shots, problems)
    title = str(answer.get("title") or "").strip() if isinstance(answer, dict) else ""
    return {"title": title[:8], "shots": out}, problems


def _check_shot(i, entry, shot, library, problems):
    kind = entry.get("kind")
    if kind not in ("story", "sublime"):
        if entry:
            problems.append(f"第 {i} 镜的 kind「{kind}」无效，按讲故事处理")
        kind = "story"
    draw = str(entry.get("draw") or "").strip()
    if not draw:
        if entry:
            problems.append(f"第 {i} 镜缺少 draw")
        draw = f"a cartoon scene that shows: {shot['text']}"
    texts = [str(t).strip()[:8] for t in entry.get("texts") or [] if str(t).strip()][:3]
    cast = [c for c in entry.get("cast") or [] if c in library and library[c]["kind"] == "cast"][:2]
    motion = entry.get("motion")
    if motion is not None and motion not in RECIPE["cardMotions"]:
        problems.append(f"第 {i} 镜的 motion「{motion}」不存在，改为随机")
        motion = None
    elements, generated = [], 0
    limits = RECIPE["elements"]
    for element in entry.get("elements") or []:
        if not isinstance(element, dict) or len(elements) >= limits["maxPerShot"]:
            continue
        chosen = element.get("motion")
        if chosen is not None and chosen not in RECIPE["elementMotions"]:
            chosen = None
        at = str(element.get("at") or "").strip()
        place = _check_place(element)
        lib = element.get("lib")
        if lib is not None:
            entry_lib = library.get(lib)
            if entry_lib is None or entry_lib["kind"] not in ("cast", "prop") or lib in cast:
                problems.append(f"第 {i} 镜的元素 {lib} 不在素材库或与主卡片重复，已去掉")
                continue
            elements.append({"lib": lib, "at": at, "motion": chosen, **place})
            continue
        element_draw = str(element.get("draw") or "").strip()
        if not element_draw or generated >= limits["maxGeneratedPerShot"]:
            continue
        generated += 1
        words = [str(t).strip()[:6] for t in element.get("texts") or [] if str(t).strip()][:2]
        elements.append({"draw": element_draw, "texts": words, "at": at, "motion": chosen, **place})
    return {"kind": kind, "draw": draw, "texts": texts, "cast": cast, "motion": motion,
            "elements": elements if kind == "story" else [], "keptElements": elements}


def _check_place(element):
    """The model's placement words for an element; anything unknown is
    dropped and the layout decides (see `element_defaults`)."""
    place = {}
    if element.get("relation") in ("beside", "above", "on"):
        place["relation"] = element["relation"]
    elif element.get("relation") == "front":
        place["relation"] = "beside"
    if element.get("size") in RECIPE["layout"]["sizes"]:
        place["size"] = element["size"]
    if element.get("side") in ("left", "right"):
        place["side"] = element["side"]
    target = element.get("target")
    if isinstance(target, int) and not isinstance(target, bool) and 0 <= target < 2:
        place["target"] = target
    return place


def _cap_sublime(plan_shots, shots, problems):
    """Sublime shots may cover at most `maxShare` of the film. The ones at
    the end are kept first (the uplift belongs there); earlier ones become
    story shots again, with the pictures the model described for them."""
    limit = RECIPE["sublime"]["maxShare"] * sum(s["end"] - s["start"] for s in shots)
    used = 0.0
    for i in range(len(shots) - 1, -1, -1):
        entry = plan_shots[i]
        if entry["kind"] != "sublime":
            continue
        # A line that tells the story (someone does or says something) is not
        # an uplift, whatever the model thought.
        named = next((n for n in RECIPE["sublime"]["notWithNames"] if n in shots[i]["text"]), None)
        if named:
            entry["kind"] = "story"
            entry["elements"] = entry["keptElements"]
            problems.append(f"第 {i} 镜改回讲故事：升华台词里有角色「{named}」，是在讲故事")
            continue
        length = shots[i]["end"] - shots[i]["start"]
        if used + length <= limit + 1e-6:
            used += length
        else:
            entry["kind"] = "story"
            entry["elements"] = entry["keptElements"]
            problems.append(f"第 {i} 镜改回讲故事：升华超过全片 {int(RECIPE['sublime']['maxShare'] * 100)}%")
    for entry in plan_shots:
        entry.pop("keptElements", None)


# ---------------------------------------------------------------- motions (seeded)

def motion_budget(story_shots):
    """How many card motions the model may choose; the rest are random."""
    return max(1, int(-(-story_shots * RECIPE["cardChosenShare"] // 1)))


def choose_motions(plan, rng):
    """Fill every story shot's card motion and every element's motion. The
    model's card choices are for special pictures only: at most
    `motion_budget`, never the same motion twice in a row; the rest come
    from the recipe's weights. The opening picture grows in."""
    names = list(RECIPE["cardRandom"])
    weights = [RECIPE["cardRandom"][n] for n in names]
    budget = motion_budget(sum(1 for s in plan["shots"] if s["kind"] == "story"))
    last = None
    for k, shot in enumerate(plan["shots"]):
        if shot["kind"] != "story":
            last = None
            continue
        if shot["motion"] is not None and k > 0:
            if shot["motion"] == last or budget <= 0:
                shot["motion"] = None
            else:
                budget -= 1
        if k == 0:
            shot["motion"] = "grow"
        elif shot["motion"] is None:
            pool = [(n, w) for n, w in zip(names, weights) if n != last]
            shot["motion"] = rng.choices([n for n, _ in pool], [w for _, w in pool])[0]
        last = shot["motion"]
        for element in shot["elements"]:
            if element["motion"] is None:
                element["motion"] = SYMBOL_MOTION.get(element.get("lib")) or rng.choice(
                    RECIPE["elementRandom"][_element_kind(element)])


def _element_kind(element):
    lib = element.get("lib")
    if lib is None:
        return "generated"
    entry = LIBRARY.get(lib, {})
    if entry.get("float"):
        return "float"
    return entry.get("kind", "prop")


# ---------------------------------------------------------------- pictures

def card_prompt(draw, texts, style):
    if texts:
        words = "; ".join(f"“{t}”" for t in texts)
        writing = f" Exactly these Chinese words appear in the picture, clearly written: {words}."
    else:
        writing = " No text or letters anywhere."
    return f"{draw.strip().rstrip('.')}.{writing} {style}"


def redraws(value):
    """`shot/3/card:1,shot/7/el/0:2` → {"shot/3/card": 1, "shot/7/el/0": 2}:
    draw that picture again as variant n."""
    out = {}
    for part in str(value or "").split(","):
        key, _, variant = part.strip().rpartition(":")
        pieces = key.split("/")
        ok = (len(pieces) == 3 and pieces[2] in ("card", "backdrop")) or (
            len(pieces) == 4 and pieces[2] == "el" and pieces[3].isdigit())
        if pieces and pieces[0] == "shot" and len(pieces) > 1 and pieces[1].isdigit() and ok:
            try:
                out[key] = max(0, int(variant))
            except ValueError:
                continue
    return out


def picture_requests(ctx, plan, library, variants):
    """Every picture to generate: {key: request}. Story cards use the shot's
    characters as reference pictures; generated elements stand alone."""
    card, element = RECIPE["card"], RECIPE["elements"]
    backdrop = RECIPE["sublime"]["backdrop"]
    requests = {}
    for k, shot in enumerate(plan["shots"]):
        if shot["kind"] == "sublime":
            # One picture for the line's blurred background, no words in it.
            key = f"shot/{k}/backdrop"
            request = {"prompt": card_prompt(shot["draw"], [], element["style"]),
                       "size": backdrop["size"], "transparent": True, "quality": backdrop["quality"]}
            if variants.get(key):
                request["variant"] = variants[key]
            requests[key] = request
            continue
        key = f"shot/{k}/card"
        request = {"prompt": card_prompt(shot["draw"], shot["texts"], card["style"]),
                   "size": card["size"], "transparent": True, "quality": card["quality"]}
        refs = [_copy_asset(ctx, library[c]) for c in shot["cast"] if c in library]
        if refs:
            request["references"] = refs
        if variants.get(key):
            request["variant"] = variants[key]
        requests[key] = request
        for j, el in enumerate(shot["elements"]):
            if "draw" not in el:
                continue
            key = f"shot/{k}/el/{j}"
            request = {"prompt": card_prompt(el["draw"], el["texts"], element["style"]),
                       "size": element["size"], "transparent": True, "quality": element["quality"]}
            if variants.get(key):
                request["variant"] = variants[key]
            requests[key] = request
    return requests


def trim_picture(ctx, key, relative, variant=0):
    """Crop a generated picture to its visible pixels so it stands on the
    ground. Each variant has its own file, so a redrawn picture gets a new rev."""
    with Image.open(ctx.workdir / relative) as image:
        image = image.convert("RGBA")
        box = image.getchannel("A").point(lambda a: 255 if a > 16 else 0).getbbox()
        if box:
            pad = 6
            image = image.crop((max(0, box[0] - pad), max(0, box[1] - pad),
                                min(image.width, box[2] + pad), min(image.height, box[3] + pad)))
        name = key.replace("/", "-") + (f"-v{variant}" if variant else "")
        target = ctx.path(f"out/pictures/{name}.png")
        image.save(target)
    return _relative(ctx, target)


# ---------------------------------------------------------------- items

def ground_items(plan, spans, total, plate_file, black):
    """Lane 0: the plate under story shots, black under sublime shots and
    the closing second; neighbours dissolve into each other."""
    motion = RECIPE["motion"]
    segments = []
    for shot, (start, end) in zip(plan["shots"], spans):
        ground = "black" if shot["kind"] == "sublime" else "plate"
        if segments and segments[-1][2] == ground:
            segments[-1][1] = end
        else:
            segments.append([start, end, ground])
    if segments[-1][2] == "black":
        segments[-1][1] = total
    else:
        segments.append([segments[-1][1], total, "black"])
    return [_item(f"bg/{n}", _rev(ground, start, end), "visual", 0, start, end - start,
                  _file(plate_file if ground == "plate" else black),
                  transition=_motion(motion["plateIn"] if n == 0 else motion["groundCross"]))
            for n, (start, end, ground) in enumerate(segments)]


def card_layout(plan, spans):
    """Per story shot: how its card enters (crossing the previous card on
    the same lane, or on its own) and whether it leaves by itself."""
    out = {}
    shots = plan["shots"]
    for k, (shot, (start, end)) in enumerate(zip(shots, spans)):
        if shot["kind"] != "story":
            continue
        spec = RECIPE["cardMotions"][shot["motion"]]
        crossing = k > 0 and shots[k - 1]["kind"] == "story"
        leaves = k + 1 >= len(shots) or shots[k + 1]["kind"] != "story"
        out[k] = {
            "transition": _motion(spec["transition"]) if crossing else None,
            "intro": None if crossing else _motion(spec["intro"]),
            "outro": _motion(RECIPE["motion"]["outro"]) if leaves else None,
            "enter": float((spec["transition"] if crossing else spec["intro"])[1]),
        }
    return out


# ---------------------------------------------------------------- layout

def picture_shape(ctx, relative):
    """Size and visual centre of a trimmed picture: the alpha-weighted
    centroid as a fraction of its width, and how much of it is ink."""
    with Image.open(ctx.workdir / relative) as image:
        size = image.size
        alpha = image.convert("RGBA").getchannel("A").resize((48, 48))
    data = list(alpha.getdata())
    total = sum(data) or 1
    cx = sum((i % 48 + 0.5) / 48 * a for i, a in enumerate(data)) / total
    return {"size": size, "cx": cx, "ink": total / (255 * len(data))}


def element_defaults(element):
    """Relation and size class of an element the model left open: symbols
    float above the card, everything else stands beside it; sizes come from
    the library entry (characters are large)."""
    lib = LIBRARY.get(element.get("lib"), {}) if element.get("lib") else {}
    relation = element.get("relation") or ("above" if lib.get("float") else "beside")
    if lib:
        size = element.get("size") or lib.get("size") or ("large" if lib.get("kind") == "cast" else "small")
    else:
        size = element.get("size") or "medium"
    return relation, size


def compose(card, elements, mirror):
    """Lay out one story shot: the card and its elements form one group on
    the ground line. Every arrangement the model left open (which side an
    element stands on, their order, a slightly smaller card) is tried; each
    is centred by visual mass within the safe margins and scored for
    balance, margins, overlaps, scale and the shot's preferred mirror. The
    best one wins.

    `card` / each element: {"shape": picture_shape(...)}; elements also carry
    `relation` (beside | above | on), `size`, optional `side`
    (left | right) and `target` (card or element index). Returns
    {"card": box, "elements": [box, ...]} with boxes
    {"x": centre (canvas widths from the middle), "bottom", "height" (canvas
    heights from the top)}."""
    spec = RECIPE["layout"]
    open_sides = [j for j, el in enumerate(elements)
                  if el["relation"] == "beside" and el.get("side") is None]
    best = None
    for sides in _side_choices(len(open_sides)):
        chosen = [dict(el) for el in elements]
        for j, side in zip(open_sides, sides):
            chosen[j]["side"] = side
        for order in (False, True):
            for card_scale in spec["cardScales"]:
                layout = _arrange(card, chosen, card_scale, order)
                if layout is None:
                    continue
                # Too wide for the frame: the whole group shrinks evenly.
                left, right = _extent(layout)
                room = 1.0 - 2 * spec["margin"]
                if right - left > room:
                    layout = _arrange(card, chosen, card_scale, order, fit=room / (right - left))
                score = _score(layout, chosen, card_scale, mirror)
                if best is None or score < best[0]:
                    best = (score, layout)
    return best[1]


def _side_choices(count):
    if count == 0:
        return [()]
    rest = _side_choices(count - 1)
    return [(side,) + r for side in ("right", "left") for r in rest]


def _width(shape, height):
    w, h = shape["size"]
    return (w / h if h else 1.0) * height / CANVAS_ASPECT


def _height_for(shape, height, max_width):
    """`height`, unless that makes the picture wider than `max_width`."""
    width = _width(shape, height)
    return height * min(1.0, max_width / width) if width > 0 else height


def _arrange(card, elements, card_scale, swap, fit=1.0):
    """Place the ground row (left elements, card, right elements) with even
    gaps, then the stacked and floating elements on their targets."""
    spec = RECIPE["layout"]
    ground = spec["ground"]
    grounded = {j for j, el in enumerate(elements) if el["relation"] == "beside"}
    base = spec["cardHeight"] if not grounded else spec["cardHeightWithElements"]
    card_h = _height_for(card["shape"], base * card_scale * fit, spec["cardMaxWidth"] * fit)
    boxes = [None] * len(elements)
    unit = spec["characterHeight"]
    for j, el in enumerate(elements):
        height = unit * spec["sizes"][el["size"]] * fit
        boxes[j] = {"height": _height_for(el["shape"], height, spec["elementMaxWidth"] * fit)}
    # The ground row, left to right.
    left = [j for j in sorted(grounded) if elements[j]["side"] == "left"]
    right = [j for j in sorted(grounded) if elements[j]["side"] == "right"]
    if swap:
        if len(left) < 2 and len(right) < 2:
            return None
        left, right = left[::-1], right[::-1]
    row = [("el", j) for j in left[::-1]] + [("card", None)] + [("el", j) for j in right]
    x, placed = 0.0, {}
    for n, (kind, j) in enumerate(row):
        shape = card["shape"] if kind == "card" else elements[j]["shape"]
        height = card_h if kind == "card" else boxes[j]["height"]
        width = _width(shape, height)
        if n:
            x += spec["gap"] * fit
        placed[(kind, j)] = (x + width / 2, width, height)
        x += width
    card_x, card_w, _ = placed[("card", None)]
    out_card = {"x": card_x, "bottom": ground, "height": card_h, "width": card_w}
    for (kind, j), (cx, width, _) in placed.items():
        if kind == "el":
            boxes[j].update({"x": cx, "width": width, "bottom": ground})
    # Stacked elements sit on their target's middle.
    for j, el in enumerate(elements):
        if el["relation"] != "on":
            continue
        anchor = boxes[el["target"]]
        height = min(boxes[j]["height"], anchor["height"] * 0.8)
        boxes[j].update({"x": anchor["x"], "width": _width(el["shape"], height), "height": height,
                         "bottom": anchor["bottom"] - anchor["height"] / 2 + height / 2})
    # Floating elements share the room above their target, side by side.
    above = {}
    for j, el in enumerate(elements):
        if el["relation"] == "above":
            target = el.get("target")
            above.setdefault(target if isinstance(target, int) else None, []).append(j)
    for target, members in above.items():
        anchor = out_card if target is None else boxes[target]
        anchor_top = anchor["bottom"] - anchor["height"]
        room = anchor_top - spec["floatGap"] - spec["top"]
        height = min(max(boxes[j]["height"] for j in members), room)
        if height < spec["floatMin"]:
            # No room overhead: hang beside the target's upper corner instead.
            height = min(max(boxes[j]["height"] for j in members), spec["floatCorner"] * fit)
            side = 1 if not right else -1
            cx = anchor["x"] + side * (anchor["width"] / 2 + spec["gap"] * fit)
            for j in members:
                width = _width(elements[j]["shape"], height)
                boxes[j].update({"x": cx + side * width / 2, "width": width, "height": height,
                                 "bottom": anchor_top + height})
                cx += side * (width + spec["gap"] * fit)
            continue
        widths = [_width(elements[j]["shape"], height) for j in members]
        span = sum(widths) + spec["gap"] * fit * (len(members) - 1)
        toward = {"right": 1, "left": -1}.get(elements[members[0]].get("side"), 0)
        cx = anchor["x"] + toward * anchor["width"] * 0.25 - span / 2
        for j, width in zip(members, widths):
            boxes[j].update({"x": cx + width / 2, "width": width, "height": height,
                             "bottom": anchor_top - spec["floatGap"]})
            cx += width + spec["gap"] * fit
    return {"card": out_card, "elements": boxes}


def _shift(layout, dx):
    for box in [layout["card"]] + layout["elements"]:
        box["x"] += dx


def _extent(layout):
    boxes = [layout["card"]] + layout["elements"]
    return (min(b["x"] - b["width"] / 2 for b in boxes), max(b["x"] + b["width"] / 2 for b in boxes))


def _score(layout, elements, card_scale, mirror):
    """Lower is better. Centres the group by visual mass (within the
    margins) as a side effect."""
    spec = RECIPE["layout"]
    margin = spec["margin"]
    boxes = [(layout["card"], None)] + [(b, el) for b, el in zip(layout["elements"], elements)]
    shapes = [None] + [el["shape"] for el in elements]
    mass = sum(b["width"] * b["height"] * (s["ink"] if s else 0.6) for (b, _), s in zip(boxes, shapes))
    centre = sum((b["x"] - b["width"] / 2 + b["width"] * (s["cx"] if s else 0.5))
                 * b["width"] * b["height"] * (s["ink"] if s else 0.6)
                 for (b, _), s in zip(boxes, shapes)) / (mass or 1)
    left, right = _extent(layout)
    # Centre the mass, but keep both margins.
    dx = -centre
    dx = min(dx, 0.5 - margin - right) if right + dx > 0.5 - margin else dx
    dx = max(dx, -0.5 + margin - left) if left + dx < -0.5 + margin else dx
    _shift(layout, dx)
    left, right = _extent(layout)
    score = 0.0
    overflow = max(0.0, -0.5 + margin - left) + max(0.0, right - 0.5 + margin)
    score += spec["weights"]["margin"] * overflow
    score += spec["weights"]["balance"] * abs(centre + dx)
    score += spec["weights"]["smaller"] * (1.0 - card_scale)
    # Overlaps: only a stacked element and its own target may overlap.
    for a in range(len(boxes)):
        for b in range(a + 1, len(boxes)):
            first, el_a = boxes[a]
            second, el_b = boxes[b]
            def on(el, index):
                return el is not None and el["relation"] == "on" and el.get("target") == index - 1
            if on(el_b, a) or on(el_a, b):
                continue
            score += spec["weights"]["overlap"] * _overlap(first, second)
    # Variety: this shot prefers its card on one side of the group.
    card_side = "left" if layout["card"]["x"] < sum(_extent(layout)) / 2 - 0.02 else \
        "right" if layout["card"]["x"] > sum(_extent(layout)) / 2 + 0.02 else "middle"
    if card_side not in ("middle", mirror):
        score += spec["weights"]["mirror"]
    return score


def _overlap(a, b):
    """Overlapping area of two boxes, as a share of the smaller one."""
    w = min(a["x"] + a["width"] / 2, b["x"] + b["width"] / 2) - max(a["x"] - a["width"] / 2,
                                                                    b["x"] - b["width"] / 2)
    h = min(a["bottom"], b["bottom"]) - max(a["bottom"] - a["height"], b["bottom"] - b["height"])
    if w <= 0 or h <= 0:
        return 0.0
    smaller = min(a["width"] * a["height"], b["width"] * b["height"])
    return w * h / smaller if smaller else 0.0


def _transform(shape, box):
    return {"x": round(box["x"], 4), "y": round(box["bottom"] - box["height"] / 2 - 0.5, 4),
            "scale": round(_fit_scale(shape["size"], box["height"]), 4)}


def card_item(k, shot, span, file, shape, box, layout):
    start, end = span
    transform = _transform(shape, box)
    return _item(f"shot/{k}/card", _rev(file, start, end, shot["motion"], transform), "visual", 1,
                 start, end - start, _file(file), transform=transform,
                 transition=layout["transition"], intro=layout["intro"], outro=layout["outro"],
                 link="voice")


def element_spans(spans, layout):
    """Where a story shot's elements may stand: from the shot's start until
    the next card starts crossing in (the crossing window is centred on the
    cut), so the elements are gone before the next picture arrives."""
    out = {}
    for k in layout:
        start, end = spans[k]
        following = layout.get(k + 1)
        if following and following["transition"]:
            end -= following["transition"]["duration"] / 2
        out[k] = (start, round(end, 3))
    return out


def element_times(shot, span, offset, chars, enter):
    """When each extra element appears: when its words are spoken, never
    before the card has landed, a calm beat after the previous one. Words
    said late in the line bring the element in a little earlier so it stays
    `minShow` on screen; one that cannot fit at all gets None (dropped)."""
    limits = RECIPE["elements"]
    start, end = span
    earliest = start + max(limits["lead"], enter + 0.2)
    times, previous = [], None
    for j, el in enumerate(shot["elements"]):
        spoken = word_time(chars, shot["audio"], el.get("at"))
        t = offset + spoken if spoken is not None else earliest + j * limits["stagger"]
        floor = earliest if previous is None else previous + limits["stagger"]
        t = min(max(t, floor), end - limits["minShow"])
        if t < floor:
            times.append(None)
            continue
        times.append(round(t, 3))
        previous = t
    return times


def calm_elements(plan, spans, stage, offset, chars, layout):
    """Keep only the elements there is time to see: none in a short shot, at
    most one or two by the shot's length, each on screen `minShow`. Returns
    the appearance times of the kept ones; dropped ones are never drawn."""
    limits = RECIPE["elements"]
    times = {}
    for k, shot in enumerate(plan["shots"]):
        if shot["kind"] != "story":
            continue
        length = spans[k][1] - spans[k][0]
        allowed = sum(1 for n, need in limits["minShot"].items() if length >= need)
        _keep_elements(shot, range(min(allowed, len(shot["elements"]))))
        at = element_times(shot, stage[k], offset, chars, layout[k]["enter"])
        _keep_elements(shot, [j for j, t in enumerate(at) if t is not None])
        times[k] = [t for t in at if t is not None]
    return times


def _keep_elements(shot, keep):
    """Keep elements by index; targets follow their element or fall away."""
    keep = list(keep)
    renumber = {old: new for new, old in enumerate(keep)}
    kept = []
    for old in keep:
        el = dict(shot["elements"][old])
        if isinstance(el.get("target"), int):
            el["target"] = renumber.get(el["target"])
        kept.append(el)
    shot["elements"] = kept


def element_item(k, j, shot, span, at, file, shape, box, card_box, lane):
    """One extra element. Stacked and floating elements take a higher lane
    than what they hang on; a character walking in comes from its own side."""
    el = shot["elements"][j]
    transform = _transform(shape, box)
    motion = RECIPE["elementMotions"][el["motion"]]["motion"]
    if el["motion"] == "walkIn" and box["x"] < card_box["x"]:
        motion = ["sb.elWalkLeft", motion[1]]
    end = span[1]
    return _item(f"shot/{k}/el/{j}", _rev(file, at, end, el["motion"], transform), "visual", lane,
                 at, end - at, _file(file), transform=transform, intro=_motion(motion),
                 outro=_motion(RECIPE["elements"]["outro"]), link="voice")


def shot_items(ctx, k, shot, span, stage, times, files, layout, mirror):
    """Card and elements of one story shot, laid out together once all of
    its pictures are known. `files`: {"card": path | None, j: path | None}."""
    if files.get("card") is None:
        return []
    card = {"shape": picture_shape(ctx, files["card"])}
    kept, index = [], []
    for j, el in enumerate(shot["elements"]):
        if files.get(j) is None:
            continue
        relation, size = element_defaults(el)
        kept.append({**el, "relation": relation, "size": size,
                     "shape": picture_shape(ctx, files[j])})
        index.append(j)
    # Targets point at element numbers of the plan; renumber to the kept ones.
    for el in kept:
        target = el.get("target")
        el["target"] = index.index(target) if isinstance(target, int) and target in index else None
        if el["relation"] == "on" and (el["target"] is None
                                       or kept[el["target"]]["relation"] != "beside"):
            el["relation"] = "beside"
        if el["relation"] == "above" and el["target"] is not None \
                and kept[el["target"]]["relation"] != "beside":
            el["target"] = None
    placed = compose(card, kept, mirror)
    items = [card_item(k, shot, span, files["card"], card["shape"], placed["card"], layout)]
    # Ground elements first, then what floats or stacks on them.
    order = sorted(range(len(kept)), key=lambda n: kept[n]["relation"] in ("above", "on"))
    for rank, n in enumerate(order):
        j = index[n]
        items.append(element_item(k, j, shot, stage, times[j], files[j], kept[n]["shape"],
                                  placed["elements"][n], placed["card"], 2 + rank))
    return items


def sublime_backdrop(ctx, k, relative):
    """The uplift background: the generated picture centred on black,
    blurred and darkened (with a vignette) so the red line reads on top."""
    from PIL import ImageDraw, ImageEnhance, ImageFilter
    spec = RECIPE["sublime"]["backdrop"]
    width, height = 1920, 1080
    canvas = Image.new("RGB", (width, height), (0, 0, 0))
    with Image.open(ctx.workdir / relative) as picture:
        picture = picture.convert("RGBA")
        scale = min(height * spec["height"] / picture.height, width * spec["maxWidth"] / picture.width)
        size = (max(1, int(picture.width * scale)), max(1, int(picture.height * scale)))
        picture = picture.resize(size, Image.LANCZOS)
        canvas.paste(picture, ((width - size[0]) // 2, (height - size[1]) // 2), picture)
    canvas = canvas.filter(ImageFilter.GaussianBlur(spec["blur"]))
    canvas = ImageEnhance.Brightness(canvas).enhance(spec["brightness"])
    # Vignette: the edges fall back to black.
    mask = Image.new("L", (width // 8, height // 8), 0)
    ImageDraw.Draw(mask).ellipse((-width // 40, -height // 30, width // 8 + width // 40,
                                  height // 8 + height // 30), fill=255)
    mask = mask.filter(ImageFilter.GaussianBlur(14)).resize((width, height), Image.BILINEAR)
    canvas = Image.composite(canvas, Image.new("RGB", (width, height), (0, 0, 0)), mask)
    # Named after the picture, so a redrawn one is a new file (and rev).
    target = ctx.path(f"out/pictures/{Path(relative).stem}-blurred.jpg")
    canvas.save(target, quality=90)
    return _relative(ctx, target)


def sublime_items(plan, spans):
    spec = RECIPE["sublime"]
    out = []
    for k, (shot, (start, end)) in enumerate(zip(plan["shots"], spans)):
        if shot["kind"] != "sublime":
            continue
        line = _spoken_line(shot["audio"]["text"], spec["lineChars"])
        widest = max(len(part) for part in line.split("\n"))
        size = round(min(spec["size"], 0.9 * CANVAS_ASPECT / max(1, widest)), 4)
        # Typed character by character, like the opening title.
        typing = {"kind": "perChar", "charDuration": 0.12,
                  "duration": round(min(1.2, len(_spoken(line)) * spec["charSec"]), 3)}
        content = _text(line, spec["style"], size, line_height=1.35, animation=typing)
        out.append(_item(f"shot/{k}/line", _rev(line, start, end, size, spec["style"]), "text", 1,
                         start + 0.15, end - start - 0.15, content,
                         transform={"x": 0.0, "y": spec["y"]}, outro=_motion(spec["outro"]),
                         link="voice"))
    return out


def backdrop_item(k, span, file):
    """The blurred picture behind a sublime line: fades in and slowly
    pushes in for the whole shot."""
    start, end = span
    spec = RECIPE["sublime"]
    zoom = {"kind": spec["backdrop"]["motion"], "duration": round(end - start, 3)}
    return _item(f"shot/{k}/backdrop", _rev(file, start, end), "visual", 1, start, end - start,
                 _file(file), intro=zoom, outro=_motion(spec["outro"]), link="voice")


def _spoken_line(text, per_line):
    """The narration as it is said, without the closing stop; long lines break
    at a comma near the middle (else at the middle)."""
    line = str(text).strip().rstrip("。！!．.")
    if len(_spoken(line)) <= per_line:
        return line
    middle = len(line) / 2
    breaks = [i for i, c in enumerate(line[:-1]) if c in "。！？!?，,、；;："]
    cut = min(breaks, key=lambda i: abs(i - middle)) + 1 if breaks else int(middle)
    if abs(cut - middle) > len(line) / 3:
        cut = int(middle)
    return line[:cut].strip().rstrip("，,、；;：。") + "\n" + line[cut:].strip("，, ")


def subtitle_items(cues, offset, quiet):
    """Cues timed by the words; a short breath between two cues keeps the
    previous one on screen. No subtitle over a sublime shot (`quiet` spans):
    the yellow line is the words there."""
    sub = RECIPE["text"]["subtitle"]
    hold = float(sub.get("holdGap", 1.0))
    longest = float(sub.get("holdMax", 7.0))
    cues = [c for c in cues if str(c.get("text") or "").strip()]
    out = []
    for index, cue in enumerate(cues):
        words = str(cue.get("text") or "").strip()
        start, end = float(cue["start"]) + offset, float(cue["end"]) + offset
        middle = (start + end) / 2
        if any(a <= middle < b for a, b in quiet):
            continue
        if index + 1 < len(cues):
            following = float(cues[index + 1]["start"]) + offset
            if 0.0 < following - end <= hold and following - start <= longest:
                end = following
        for a, b in quiet:
            if start < a < end:
                end = a
        out.append(_item(f"sub/{index}", _rev(words, start, end), "subtitle", 0, start,
                         max(0.2, end - start), _text(words, sub["style"], sub["size"]),
                         transform={"x": 0.0, "y": sub["y"]}, link="voice"))
    return out


def bgm_items(ctx, total):
    bgm = RECIPE["bgm"]
    out = []
    for index, part in enumerate(bgm["parts"]):
        source = PACKAGE / "assets" / "bgm" / part["file"]
        if not source.is_file():
            continue
        target = ctx.path(f"out/bgm-{index}.mp3")
        if not target.is_file():
            shutil.copyfile(source, target)
        start, end = part["from"] * total, part["to"] * total
        out.append(_item(f"bgm/{index}", _rev(part["file"], start, end), "music", index + 1,
                         start, end - start, _file(_relative(ctx, target)), gain=bgm["gain"],
                         fade={"in": 0.0 if index == 0 else bgm["fade"], "out": bgm["fade"]}))
    return out


def title_items(title, black):
    opening = RECIPE["opening"]
    offset = float(RECIPE["timing"]["titleSec"])
    card_duration = min(offset, float(opening.get("cardDuration", offset)))
    items = [_item("title/card", _rev("black", card_duration), "visual", 0, 0.0, card_duration,
                   _file(black), intro=_motion(opening.get("cardIntro")))]
    if title:
        start = max(0.0, float(opening.get("textStart", 0.1)))
        duration = max(0.05, float(opening.get("textDuration", offset - start)))
        content = _text(title, opening["style"], float(opening["size"]),
                        letter_spacing=float(opening.get("letterSpacing", 0.0)),
                        line_height=float(opening.get("lineHeight", 1.15)),
                        animation=opening.get("textAnimation"))
        items.append(_item("title/text", _rev(title, opening["style"], content, start, duration),
                           "text", 1, start, duration, content,
                           transform={"x": 0.0, "y": float(opening.get("y", 0.0))},
                           outro=_motion(opening.get("outro"))))
    return items


# ---------------------------------------------------------------- run

LIBRARY = {}


class OfflineContext:
    """Answers service calls from `fixture.json` in the workdir (copied from
    `tests/` by the host self-check), so the package can be checked without
    network or keys. Everything else goes to the real context."""

    def __init__(self, ctx):
        self._ctx = ctx
        local = ctx.workdir / "fixture.json"
        source = local if local.is_file() else PACKAGE / "tests" / "fixture.json"
        self._fixture = json.loads(source.read_text(encoding="utf-8"))

    def __getattr__(self, name):
        return getattr(self._ctx, name)

    def call_async(self, service, request):
        from concurrent.futures import Future
        future = Future()
        try:
            future.set_result(self.call(service, request))
        except BaseException as error:  # the caller sees it on result()
            future.set_exception(error)
        return future

    def call(self, service, request):
        if service == "speech":
            wav = self._ctx.path("services/fixture-voice.wav")
            if not wav.is_file():
                import wave
                with wave.open(str(wav), "wb") as out:
                    out.setnchannels(1)
                    out.setsampwidth(2)
                    out.setframerate(24000)
                    out.writeframes(b"\0\0" * int(24000 * self._fixture["speech"]["duration"]))
            return {**self._fixture["speech"], "file": "services/fixture-voice.wav"}
        if service == "subtitles":
            return {"cues": self._fixture["cues"]}
        if service == "chat":
            key = "script" if "口播文案" in request["messages"][0]["content"] else "plan"
            value = self._fixture[key]
            return {"text": value if isinstance(value, str) else json.dumps(value, ensure_ascii=False)}
        if service == "image":
            out = self._ctx.path(f"services/fixture-{_rev(request['prompt'], request.get('variant', 0))}.png")
            if not out.is_file():
                Image.new("RGBA", (512, 512), (240, 160, 40, 255)).save(out)
            return {"file": _relative(self._ctx, out), "width": 512, "height": 512}
        raise RunFailed(f"离线样例没有 {service} 的答案", "invalid_input")


@worker.op("run")
def run(ctx):
    global LIBRARY
    if isinstance(ctx.input, dict) and ctx.input.get("offline"):
        ctx = OfflineContext(ctx)
    params = ctx.params if isinstance(ctx.params, dict) else {}
    value = ctx.input if isinstance(ctx.input, dict) else {}
    script = " ".join(str(value.get("script", "")).split())[:6000]
    topic = str(params.get("topic") or "").strip()
    seconds = float(params.get("seconds") or 30)
    library = LIBRARY = _library()
    if not library:
        raise RunFailed("素材库为空：缺少 assets/library/library.json", "invalid_input")

    # Progress phase = graph.json node id (workflow panel keys off this).
    ctx.progress("script", 0.02)
    rate = int(params.get("rate", RECIPE["speech"]["rate"]))
    if not script:
        if not topic:
            raise RunFailed("请填写口播文案，或在参数里填写主题", "invalid_input")
        script = write_script(ctx, topic, seconds, rate)
    ctx.path("out/script.txt").write_text(script, encoding="utf-8")

    # Fast parts first: voice, music and the opening go on the timeline now.
    ctx.progress("speech", 0.1)
    speech = ctx.call("speech", {
        "text": script,
        "voice": params.get("voice") or "S_RksT6Z6g2",
        "rate": rate,
    })
    timeline = speech.get("timeline") or {}
    voice_end = float(speech.get("duration") or timeline.get("duration") or 1.0)
    sentences = [s for s in timeline.get("sentences") or [] if str(s.get("text", "")).strip()]
    if not sentences:
        raise RunFailed("配音没有返回句子时间", "internal")
    offset = float(RECIPE["timing"]["titleSec"])
    shots_end = offset + voice_end + RECIPE["timing"]["tailSec"]
    total = shots_end + RECIPE["timing"]["endSec"]
    black = _solid(ctx, "black", (0, 0, 0))
    plate = library.get(RECIPE["plate"])
    plate_file = _copy_asset(ctx, plate) if plate else black
    first = [_item("voice", _rev(speech["file"]), "voice", 0, offset, voice_end, _file(speech["file"]))]
    first += bgm_items(ctx, total)
    if topic:
        first += title_items(topic[:8], black)
    ctx.emit(first)

    ctx.progress("first_emit", 0.15)
    # Continues into subtitles / split / label below; report storyboard cluster.
    ctx.progress("subtitles", 0.18)
    cues = ctx.call("subtitles", {"script": script, "speech": speech["file"],
                                  "maxChars": RECIPE["text"]["subtitle"]["maxChars"]}).get("cues") or []
    shots = split_shots(sentences, cues)
    plan, problems = label_shots(ctx, shots, library)
    for entry, shot in zip(plan["shots"], shots):
        entry["audio"] = shot
    rng = random.Random(_rev(script, getattr(ctx, "seed", 0)))
    choose_motions(plan, rng)
    spans = shot_spans(shots, offset, voice_end)
    ctx.path("out/plan.json").write_text(json.dumps({**plan, "problems": problems},
                                                    ensure_ascii=False, indent=1), encoding="utf-8")

    quiet = [span for entry, span in zip(plan["shots"], spans) if entry["kind"] == "sublime"]
    words = ground_items(plan, spans, total, plate_file, black)
    words += subtitle_items(cues, offset, quiet)
    words += sublime_items(plan, spans)
    if not topic:
        words += title_items(str(plan.get("title") or RECIPE["opening"].get("defaultText", ""))[:8], black)
    ctx.emit(words)

    ctx.progress("split_shots", 0.22)
    ctx.progress("label_shots", 0.24)
    ctx.progress("choose_motions", 0.26)
    ctx.progress("ground_emit", 0.28)
    ctx.progress("picture_batch", 0.3)
    chars = timeline.get("chars") or []
    layout = card_layout(plan, spans)
    stage = element_spans(spans, layout)
    at = calm_elements(plan, spans, stage, offset, chars, layout)
    # Story shots alternate which side of the group their card prefers.
    side = rng.choice(["left", "right"])
    mirror = {}
    for k in at:
        mirror[k] = side
        side = "right" if side == "left" else "left"
    # A shot goes on the timeline, laid out as a whole, once all of its
    # pictures are known; library elements are known at once.
    files, missing = {}, {}
    for k in at:
        files[k] = {}
        missing[k] = {"card"}
        for j, el in enumerate(plan["shots"][k]["elements"]):
            if "lib" in el:
                files[k][j] = _copy_asset(ctx, library[el["lib"]])
            else:
                missing[k].add(j)

    variants = redraws(params.get("redraw"))
    requests = picture_requests(ctx, plan, library, variants)
    pending = {ctx.call_async("image", request): key for key, request in requests.items()}
    placed, done = 0, 0
    for future in as_completed(pending):
        key = pending[future]
        parts = key.split("/")
        k = int(parts[1])
        shot = plan["shots"][k]
        done += 1
        ctx.progress("picture_batch", 0.3 + 0.65 * done / max(1, len(pending)))
        if parts[2] == "backdrop":
            # Without its picture a sublime shot simply stays black.
            try:
                picture = trim_picture(ctx, key, future.result()["file"], variants.get(key, 0))
                ctx.emit([backdrop_item(k, spans[k], sublime_backdrop(ctx, k, picture))])
                placed += 1
            except (ServiceError, RunFailed, KeyError, OSError) as error:
                problems.append(f"{key} 生成失败：{error}")
            continue
        slot = "card" if parts[2] == "card" else int(parts[3])
        try:
            file = trim_picture(ctx, key, future.result()["file"], variants.get(key, 0))
        except (ServiceError, RunFailed, KeyError, OSError) as error:
            problems.append(f"{key} 生成失败：{error}")
            file = None
            if slot == "card":
                # A card that failed every attempt: the shot's character from
                # the library stands in, so the shot is not empty.
                cast = next((c for c in shot["cast"] if c in library), None)
                file = _copy_asset(ctx, library[cast]) if cast else None
        files[k][slot] = file
        missing[k].discard(slot)
        if missing[k]:
            continue
        items = shot_items(ctx, k, shot, spans[k], stage[k], at[k], files[k], layout[k], mirror[k])
        if items:
            ctx.emit(items)
            placed += len(items)

    ctx.progress("done", 1.0)
    lengths = sorted(e - s for s, e in spans)
    story = [s for s in plan["shots"] if s["kind"] == "story"]
    sublime = sum(e - s for (s, e), entry in zip(spans, plan["shots"]) if entry["kind"] == "sublime")
    return {
        "recipe": VERSION,
        "duration": round(total, 3),
        "shots": len(plan["shots"]),
        "medianShot": round(lengths[len(lengths) // 2], 2) if lengths else 0,
        "sublimeShare": round(sublime / max(1e-6, spans[-1][1] - spans[0][0]), 3),
        "elements": sum(len(s["elements"]) for s in story),
        "cardMotions": [s["motion"] for s in story],
        "placedVisuals": placed,
        "planProblems": problems,
    }


if __name__ == "__main__":
    worker.serve()
