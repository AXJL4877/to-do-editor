"""海绵宝宝口播式成片流水线（可复用）。

    python scripts/koubo.py --stage script     # 文案（chat 生成，可手改 prod/script.txt）
    python scripts/koubo.py --stage tts        # 克隆音色口播 + 逐字时间戳
    python scripts/koubo.py --stage plan       # 句子 → 镜头计划（可手改 prod/plan.json）
    python scripts/koubo.py --stage scenes     # 按计划生成镜头画面（缺图会自动跳过）
    python scripts/koubo.py --stage project    # 装配编辑器工程
    python scripts/koubo.py --stage export     # 用编辑器渲染器导出 MP4
    python scripts/koubo.py --stage all        # 全流程

配置：scripts/koubo.json（见 DEFAULT）。所有中间产物在 target/prod/ 下。
"""
import argparse
import base64
import json
import os
import shutil
import subprocess
import wave
import sys
import time
import urllib.request

from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PROD = os.path.join(ROOT, "target", "prod")
SENTENCES = os.path.join(PROD, "voice.json")
PLAN = os.path.join(PROD, "plan.json")
SCRIPT_TXT = os.path.join(PROD, "script.txt")
VOICE = os.path.join(PROD, "voice.wav")
SCENES = os.path.join(PROD, "scenes")

DEFAULT = {
    "project": "loss-aversion",
    "title": "损失厌恶",
    "topic": "用海绵宝宝世界解释「损失厌恶」，场景设在一家海底快餐店",
    "style": "海绵宝宝动画风格，扁平卡通，粗黑描边，明亮饱和，蓝绿色海底配色，干净色块，画面中不要出现文字",
    "voice": {"speaker": "S_RksT6Z6g2", "resource_id": "seed-icl-2.0", "speech_rate": 16},
    "text": {
        "sub_size": 0.062, "sub_y": 0.40,
        "title_size": 0.13, "title_y": -0.06,
        "label_size": 0.050, "label_y": -0.31,
    },
    "bgm": [
        {"file": "extensions/builtin/spongebob-koubo/assets/bgm/intro-holding-court.mp3",
         "start": 0.0, "end": 20.0, "gain": 0.22},
        {"file": "extensions/builtin/spongebob-koubo/assets/bgm/mid-jingwei.mp3",
         "start": 18.0, "end": 38.0, "gain": 0.22},
        {"file": "extensions/builtin/spongebob-koubo/assets/bgm/outro-memories.mp3",
         "start": 36.0, "end": None, "gain": 0.22},
    ],
    "plate": ("D:/Desktop/TO-DO-App/todo-editor/.data/skills/studio.spongebob-koubo"
              "/workflow/kit/assets/backgrounds/plate_beach.jpg"),
    "transition": 0.06,
    "intro": {"seconds": 1.5, "text": "损失厌恶", "size": 0.40},
    "labels": [],
    "numerals": [{"shot": 2, "text": "30:00", "size": 0.20},
                 {"shot": 5, "text": "亏 ≠ 没赚", "size": 0.10}],
}

SUB_STYLE = {
    "id": "sub-white", "name": "白字黑描边",
    "font": {"family": "Noto Sans SC Bold", "file": "noto-sans-sc-bold.otf",
             "fallbacks": ["source-han-serif-sc-heavy.ttf"]},
    "layers": [
        {"kind": "fill", "paint": "#00000099", "offset": [0.03, 0.04], "blur": 0.04},
        {"kind": "stroke", "width": 0.12, "paint": "#000000FF"},
        {"kind": "fill", "paint": "#FFFFFFFF"},
    ],
}
LABEL_STYLE = {
    "id": "label-gold", "name": "金字黑描边",
    "font": {"family": "Noto Sans SC Bold", "file": "noto-sans-sc-bold.otf",
             "fallbacks": ["source-han-serif-sc-heavy.ttf"]},
    "layers": [
        {"kind": "stroke", "width": 0.14, "paint": "#000000FF"},
        {"kind": "fill", "paint": "#FFD34DFF"},
    ],
}
TITLE_STYLE = {
    "id": "title-white", "name": "白色标题",
    "font": {"family": "思源宋 Heavy", "file": "source-han-serif-sc-heavy.ttf",
             "fallbacks": ["noto-sans-sc-bold.otf"]},
    "layers": [
        {"kind": "fill", "paint": "#000000AA", "offset": [0.04, 0.05], "blur": 0.05},
        {"kind": "fill", "paint": "#FFFFFFFF"},
    ],
}
PUNCT = "，。！？；：、,.!?;:"

# 参考片开场：黑底毛笔红字，灰色重影在后、浅色描边包红字。
RED_TITLE_STYLE = {
    "id": "title-red-brush", "name": "毛笔红字",
    "font": {"family": "字魂剑气手书", "file": "jianqi-shoushu.ttf",
             "fallbacks": ["source-han-serif-sc-heavy.ttf"]},
    "layers": [
        {"kind": "fill", "paint": "#8C8C8CFF", "offset": [0.018, 0.020], "blur": 0.004,
         "scale": 1.0},
        {"kind": "stroke", "width": 0.045, "paint": "#EFEADCFF"},
        {"kind": "fill", "paint": "#E01B1BFF"},
    ],
}


def config():
    merged = json.loads(json.dumps(DEFAULT))
    path = os.path.join(ROOT, "scripts", "koubo.json")
    if os.path.exists(path):
        with open(path, encoding="utf-8") as handle:
            override = json.load(handle)
        for key, value in override.items():
            if isinstance(value, dict) and isinstance(merged.get(key), dict):
                merged[key].update(value)
            else:
                merged[key] = value
    return merged


def secret(name):
    """开发/CI 用环境变量；桌面端密钥在凭据管理器里，导出前先写进环境。"""
    value = os.environ.get(name)
    if not value:
        raise SystemExit(f"缺少环境变量 {name}")
    return value


def http_json(url, body, key, extra=None):
    headers = {"Content-Type": "application/json", "Authorization": f"Bearer {key}"}
    headers.update(extra or {})
    request = urllib.request.Request(url, data=json.dumps(body).encode("utf-8"), headers=headers)
    with urllib.request.urlopen(request, timeout=300) as response:
        return json.loads(response.read().decode("utf-8"))


def stage_script(cfg):
    """chat 生成口播文案；已有 script.txt 则保留（可手改）。"""
    os.makedirs(PROD, exist_ok=True)
    if os.path.exists(SCRIPT_TXT) and os.path.getsize(SCRIPT_TXT) > 0:
        print("script: keep", SCRIPT_TXT)
        return
    key = secret("TODO_HUNIU_API_KEY")
    base = os.environ.get("TODO_HUNIU_BASE_URL", "https://huniuai.com")
    model = os.environ.get("TODO_CHAT_MODEL", "deepseek-v4.1-flash")
    prompt = (
        f"写一段 45 到 60 秒的中文短视频口播文案，主题：{cfg['topic']}。"
        "要求：1) 用一个具体场景讲故事，别一上来讲概念；2) 每句 8-16 个字，句间用中文句号分隔；"
        "3) 全篇 320-380 个汉字；4) 结尾是一个短问句；5) 只输出正文，不要标题、不要序号、不要引号。"
    )
    payload = http_json(f"{base}/v1/chat/completions",
                        {"model": model, "messages": [{"role": "user", "content": prompt}],
                         "temperature": 0.8}, key)
    text = payload["choices"][0]["message"]["content"].strip()
    with open(SCRIPT_TXT, "w", encoding="utf-8") as handle:
        handle.write(text)
    print("script:", len(text), "chars ->", SCRIPT_TXT)


def stage_tts(cfg):
    """火山克隆音色：一次请求拿音频 + 逐字时间戳。"""
    text = open(SCRIPT_TXT, encoding="utf-8").read().strip()
    body = {
        "user": {"uid": "todo-editor"},
        "req_params": {
            "text": text,
            "speaker": cfg["voice"]["speaker"],
            "audio_params": {"format": "pcm", "sample_rate": 24000, "enable_subtitle": True,
                             "speech_rate": cfg["voice"]["speech_rate"]},
            "additions": json.dumps({"disable_markdown_filter": True}),
        },
    }
    request = urllib.request.Request(
        "https://openspeech.bytedance.com/api/v3/tts/unidirectional",
        data=json.dumps(body).encode("utf-8"),
        headers={"Content-Type": "application/json", "X-Api-Key": secret("TODO_VOLC_API_KEY"),
                 "X-Api-Resource-Id": cfg["voice"]["resource_id"]},
    )
    pcm, sentences = bytearray(), []
    with urllib.request.urlopen(request, timeout=300) as response:
        for raw in response:
            line = raw.decode("utf-8", "replace").strip()
            if not line or line.startswith(("event:", ":")):
                continue
            data = line[5:].strip() if line.startswith("data:") else line
            try:
                value = json.loads(data)
            except ValueError:
                continue
            if value.get("data"):
                pcm += base64.b64decode(value["data"])
            if isinstance(value.get("sentence"), dict):
                sentences.append(value["sentence"])
    if not pcm:
        raise SystemExit("tts: no audio")
    channels = bytes(pcm)
    header = b"RIFF" + (36 + len(channels)).to_bytes(4, "little") + b"WAVEfmt "
    header += (16).to_bytes(4, "little") + (1).to_bytes(2, "little")
    header += (1).to_bytes(2, "little") + (24000).to_bytes(4, "little")
    header += (48000).to_bytes(4, "little") + (2).to_bytes(2, "little")
    header += (16).to_bytes(2, "little") + b"data" + len(channels).to_bytes(4, "little")
    with open(VOICE, "wb") as handle:
        handle.write(header + channels)
    with open(SENTENCES, "w", encoding="utf-8") as handle:
        json.dump({"duration": len(pcm) / 2 / 24000, "sentences": sentences}, handle,
                  ensure_ascii=False, indent=1)
    print(f"tts: {len(pcm)/2/24000:.2f}s, {len(sentences)} sentences")


def load_sentences():
    with open(SENTENCES, encoding="utf-8") as handle:
        data = json.load(handle)
    out = []
    for sentence in data["sentences"]:
        words = sentence.get("words") or []
        if words:
            out.append({"text": sentence["text"], "start": words[0]["startTime"],
                        "end": words[-1]["endTime"], "words": words})
    return out, data["duration"]


def stage_plan(cfg):
    """句子 → 镜头：每镜 1–3 句、≥3.5 秒；画面提示词由该镜首句派生。"""
    sentences, duration = load_sentences()
    shots, current = [], []
    for index, sentence in enumerate(sentences):
        current.append(index)
        span = sentences[current[0]]["start"], sentence["end"]
        if span[1] - span[0] >= 5.0 or len(current) >= 3 or index == len(sentences) - 1:
            shots.append(current)
            current = []
    if current:
        shots.append(current)
    plan = {
        "duration": duration,
        "shots": [{
            "sentences": group,
            "scene": f"s{number + 1}",
            "prompt": f"根据这句话的画面：{sentences[group[0]]['text']}",
            "variant": ["full", "zoom", "mirror"][number % 3],
            "transform": [(0.0, 0.02, 0.78), (-0.16, 0.03, 0.74),
                          (0.16, 0.02, 0.70)][number % 3],
        } for number, group in enumerate(shots)],
    }
    plan["shots"][-1]["closing"] = True
    with open(PLAN, "w", encoding="utf-8") as handle:
        json.dump(plan, handle, ensure_ascii=False, indent=1)
    print(f"plan: {len(plan['shots'])} shots -> {PLAN}")


def stage_scenes(cfg):
    """串行生成镜头画面（网关并发会断）。已有文件跳过。"""
    plan = json.load(open(PLAN, encoding="utf-8"))
    os.makedirs(SCENES, exist_ok=True)
    key = secret("TODO_HUNIU_API_KEY")
    base = os.environ.get("TODO_HUNIU_BASE_URL", "https://huniuai.com")
    model = os.environ.get("TODO_IMAGE_MODEL", "gpt-image-2")
    for shot in plan["shots"]:
        if shot.get("closing"):
            continue
        path = os.path.join(SCENES, f"{shot['scene']}.img")
        if os.path.exists(path) and os.path.getsize(path) > 20000:
            continue
        payload = http_json(f"{base}/v1/images/generations",
                            {"model": model, "prompt": f"{cfg['style']}。{shot['prompt']}",
                             "size": "1024x1024", "n": 1}, key)
        item = (payload.get("data") or [{}])[0]
        data = (base64.b64decode(item["b64_json"]) if item.get("b64_json")
                else urllib.request.urlopen(item["url"], timeout=300).read())
        with open(path, "wb") as handle:
            handle.write(data)
        print(f"{shot['scene']}: {len(data)//1024} KB")
        time.sleep(1)


def char_times(sentence):
    text, words = sentence["text"], sentence["words"]
    known, index = {}, 0
    for pos, char in enumerate(text):
        if char in PUNCT:
            continue
        if index < len(words) and words[index]["word"] == char:
            known[pos] = (words[index]["startTime"], words[index]["endTime"])
            index += 1
    if not known:
        return [(char, sentence["start"], sentence["end"]) for char in text]
    positions = sorted(known)
    result = []
    for pos, char in enumerate(text):
        if pos in known:
            result.append((char, *known[pos]))
            continue
        before = max((p for p in positions if p < pos), default=None)
        after = min((p for p in positions if p > pos), default=None)
        if before is None:
            moment = known[after][0]
        elif after is None:
            moment = known[before][1]
        else:
            low, high = known[before][1], known[after][0]
            moment = low + (high - low) / max(1, after - before) * (pos - before)
        result.append((char, moment, moment))
    return result


def cues_for(sentences, limit=14):
    """按标点切句、再按字数上限切分；时间全部来自逐字时间戳。"""
    blocks, pending = [], []
    for sentence in sentences:
        for char, start, end in char_times(sentence):
            pending.append((char, start, end))
            if char in PUNCT:
                blocks.append(pending)
                pending = []
    if pending:
        blocks.append(pending)
    cues = []
    for block in blocks:
        body = [(char, start, end) for char, start, end in block if char not in PUNCT]
        while len(body) > limit:
            cues.append(body[:limit])
            body = body[limit:]
        if body:
            cues.append(body)
    return [{"text": "".join(char for char, _, _ in body),
             "start": max(0.0, body[0][1] - 0.04),
             "end": body[-1][2] + 0.18} for body in cues]


def trim(clips, gap=0.02, floor=0.2):
    clips.sort(key=lambda clip: clip["start"])
    for index in range(len(clips) - 1):
        current, following = clips[index], clips[index + 1]
        limit = following["start"] - gap
        if current["start"] + current["duration"] > limit:
            current["duration"] = round(max(floor, limit - current["start"]), 3)
            if current["start"] + current["duration"] > limit:
                following["start"] = round(current["start"] + current["duration"] + gap, 3)


def crop_square(image):
    width, height = image.size
    side = min(width, height)
    left, top = (width - side) // 2, (height - side) // 2
    return image.crop((left, top, left + side, top + side))


def crop_to_ratio(image, ratio):
    """居中裁到指定宽高比（16:9 的舞台卡用它铺满画布）。"""
    width, height = image.size
    if width / height > ratio:
        new_width = int(height * ratio)
        left = (width - new_width) // 2
        return image.crop((left, 0, left + new_width, height))
    new_height = int(width / ratio)
    top = (height - new_height) // 2
    return image.crop((0, top, width, top + new_height))


def stage_project(cfg):
    sentences, duration = load_sentences()
    duration = round(duration, 2)
    plan = json.load(open(PLAN, encoding="utf-8"))
    shot_list = plan["shots"]
    folder = os.path.join(ROOT, "data", cfg["project"])
    media_dir = os.path.join(folder, "media")
    os.makedirs(media_dir, exist_ok=True)

    plate = Image.open(cfg["plate"]).convert("RGB").resize((1920, 1080), Image.LANCZOS)
    plate.save(os.path.join(media_dir, "plate.png"), optimize=True)

    files = {"plate": os.path.join(media_dir, "plate.png")}
    card_ratio = 1920 / 1080
    for shot in shot_list:
        path = os.path.join(SCENES, f"{shot['scene']}.img")
        if not os.path.exists(path):
            shot["missing"] = True
            continue
        # 舞台卡按 16:9 裁切到画布尺寸：满幅铺开，消除单侧留白与出血。
        card = crop_to_ratio(Image.open(path).convert("RGB"), card_ratio).resize(
            (1920, 1080), Image.LANCZOS)
        card.save(os.path.join(media_dir, f"{shot['scene']}.png"), optimize=True)
        files[shot["scene"]] = os.path.join(media_dir, f"{shot['scene']}.png")
        width, height = card.size
        inset_x, inset_y = int(width * 0.08), int(height * 0.08)
        zoom = card.crop((inset_x, inset_y, width - inset_x, height - inset_y)).resize(
            (1920, 1080), Image.LANCZOS)
        zoom.save(os.path.join(media_dir, f"{shot['scene']}_zoom.png"), optimize=True)
        files[f"{shot['scene']}_zoom"] = os.path.join(media_dir, f"{shot['scene']}_zoom.png")
        card.transpose(Image.FLIP_LEFT_RIGHT).save(
            os.path.join(media_dir, f"{shot['scene']}_mirror.png"), optimize=True)
        files[f"{shot['scene']}_mirror"] = os.path.join(media_dir,
                                                        f"{shot['scene']}_mirror.png")

    Image.new("RGB", (1920, 1080), (0, 0, 0)).save(os.path.join(media_dir, "black.png"))
    files["black"] = os.path.join(media_dir, "black.png")
    shutil.copyfile(VOICE, os.path.join(media_dir, "voice.wav"))
    files["voice"] = os.path.join(media_dir, "voice.wav")
    for index, track in enumerate(cfg["bgm"]):
        target = os.path.join(media_dir, f"bgm{index}.mp3")
        shutil.copyfile(os.path.join(ROOT, track["file"]), target)
        files[f"bgm{index}"] = target

    bounds = [0.0]
    for previous, current in zip(shot_list, shot_list[1:]):
        bounds.append(round((sentences[previous["sentences"][-1]]["end"]
                             + sentences[current["sentences"][0]]["start"]) / 2, 3))
    bounds.append(duration)

    media, cards, lanes, texts, annotations = [], [], {"v2": [], "v3": []}, [], []

    def add_media(media_id, name, path, kind="image", width=1920, height=1080,
                  seconds=30.0, audio=False):
        if not any(item["id"] == media_id for item in media):
            media.append({"id": media_id, "name": name, "path": path.replace("\\", "/"),
                          "duration": seconds, "hasAudio": audio, "width": width,
                          "height": height, "kind": kind})

    add_media("m_plate", "背景板", files["plate"])
    plate_clip = [{"id": "bg", "mediaId": "m_plate", "start": 0.0, "duration": duration,
                   "offset": 0.0, "rate": 1.0, "intro": {"kind": "fade", "duration": 0.5}}]

    transition = float(cfg.get("transition", 0.16))
    for index, shot in enumerate(shot_list):
        start, end = bounds[index], bounds[index + 1]
        if shot.get("closing"):
            add_media("m_black", "结尾卡", files["black"])
            lanes["v2"].append({"id": f"c{index + 1:02d}", "mediaId": "m_black",
                                "start": start, "duration": round(end - start, 3),
                                "offset": 0.0, "rate": 1.0,
                                "intro": {"kind": "fade", "duration": 0.25}})
            continue
        if shot.get("missing"):
            continue
        source = shot["scene"]
        variant = shot.get("variant", "full")
        if variant != "full":
            source = f"{shot['scene']}_{variant}"
        add_media(f"m_{source}", source, files[source])
        x, y, scale = shot.get("transform", (0.0, 0.02, 0.78))
        lane = "v2" if index % 2 == 0 else "v3"
        lanes[lane].append({
            "id": f"c{index + 1:02d}", "mediaId": f"m_{source}",
            "start": round(max(0.0, start - (transition if index else 0.0)), 3),
            "duration": round(end - start + (transition if index else 0.0), 3),
            "offset": 0.0, "rate": 1.0,
            "transform": {"x": x, "y": y, "scale": scale, "scaleX": scale,
                          "scaleY": scale, "opacity": 1.0},
            "intro": {"kind": "fade" if index else "pop",
                      "duration": transition if index else 0.35},
            "outro": transition if index < len(shot_list) - 1 else 0.4,
        })

    size = cfg["text"]
    cues = cues_for(sentences)
    # 短停顿不退字幕：把当前条顺延到下一条前 0.08s（最多顺延 2.5s）
    for index in range(len(cues) - 1):
        gap = cues[index + 1]["start"] - cues[index]["end"]
        if 0.02 < gap <= 5.5 and cues[index]["end"] + gap - cues[index]["start"] <= 7.0:
            cues[index]["end"] = round(cues[index + 1]["start"] - 0.08, 3)
    for cue in cues:
        texts.append({
            "id": f"t{len(texts) + 1:03d}", "start": round(cue["start"], 3),
            "duration": round(max(0.4, cue["end"] - cue["start"]), 3),
            "transform": {"x": 0.0, "y": size["sub_y"], "scale": 1.0, "scaleX": 1.0,
                          "scaleY": 1.0, "opacity": 1.0},
            "text": {"text": cue["text"], "styleId": "sub-white",
                     "layout": {"fontSize": size["sub_size"], "letterSpacing": 0.0,
                                "lineHeight": 1.2, "align": "center"}},
        })
    for label in cfg.get("labels", []):
        index = int(label["shot"])
        if index >= len(shot_list):
            continue
        group = shot_list[index]["sentences"]
        start = round(sentences[group[0]]["start"], 3)
        annotations.append({
            "id": f"label{index}", "start": start,
            "duration": round(max(1.2, sentences[group[-1]]["end"] - start + 0.2), 3),
            "transform": {"x": 0.0, "y": size["label_y"], "scale": 1.0, "scaleX": 1.0,
                          "scaleY": 1.0, "opacity": 1.0},
            "text": {"text": label["text"], "styleId": "label-gold",
                     "layout": {"fontSize": size["label_size"], "letterSpacing": 0.02,
                                "lineHeight": 1.2, "align": "center"}},
        })
    for numeral in cfg.get("numerals", []):
        index = int(numeral["shot"])
        if index >= len(shot_list):
            continue
        group = shot_list[index]["sentences"]
        start = round(sentences[group[0]]["start"], 3)
        shot_end = bounds[index + 1]
        annotations.append({
            "id": f"big{index}", "start": start,
            "duration": round(max(1.5, min(shot_end, sentences[group[-1]]["end"]) - start), 3),
            "transform": {"x": 0.0, "y": -0.02, "scale": 1.0, "scaleX": 1.0,
                          "scaleY": 1.0, "opacity": 1.0},
            "text": {"text": numeral["text"], "styleId": "label-gold",
                     "layout": {"fontSize": numeral["size"], "letterSpacing": 0.06,
                                "lineHeight": 1.2, "align": "center"}},
        })
    closing_start = bounds[len(shot_list) - 1]
    title = [{"id": "title", "start": round(closing_start, 3),
              "duration": round(duration - closing_start, 3),
              "transform": {"x": 0.0, "y": size["title_y"], "scale": 1.0, "scaleX": 1.0,
                            "scaleY": 1.0, "opacity": 1.0},
              "text": {"text": cfg["title"], "styleId": "title-white",
                       "layout": {"fontSize": size["title_size"], "letterSpacing": 0.05,
                                  "lineHeight": 1.2, "align": "center"}}}]

    add_media("m_voice", "口播.wav", files["voice"], "audio", 0, 0, duration, True)
    voice = [{"id": "a1", "mediaId": "m_voice", "start": 0.0, "duration": duration,
              "offset": 0.0, "rate": 1.0,
              "audio": {"gain": 1.0, "fadeIn": 0.0, "fadeOut": 0.0}}]

    tracks = [
        {"id": "t1", "name": "字幕", "kind": "text", "muted": False, "locked": False,
         "clips": texts},
        {"id": "t2", "name": "标注", "kind": "text", "muted": False, "locked": False,
         "clips": annotations},
        {"id": "t3", "name": "标题", "kind": "text", "muted": False, "locked": False,
         "clips": title},
        {"id": "v2", "name": "画面A", "kind": "video", "muted": False, "locked": False,
         "clips": lanes["v2"]},
        {"id": "v3", "name": "画面B", "kind": "video", "muted": False, "locked": False,
         "clips": lanes["v3"]},
        {"id": "v1", "name": "背景", "kind": "video", "muted": False, "locked": False,
         "clips": plate_clip},
        {"id": "a1", "name": "口播", "kind": "audio", "muted": False, "locked": False,
         "clips": voice},
    ]
    for index, track in enumerate(cfg["bgm"]):
        add_media(f"m_bgm{index}", os.path.basename(track["file"]), files[f"bgm{index}"],
                  "audio", 0, 0, 120.0, True)
        end = duration if not track.get("end") else min(float(track["end"]), duration)
        tracks.append({"id": f"a{index + 2}", "name": f"BGM{index + 1}", "kind": "audio",
                       "muted": False, "locked": False,
                       "clips": [{"id": f"b{index}", "mediaId": f"m_bgm{index}",
                                  "start": float(track["start"]),
                                  "duration": round(end - float(track["start"]), 2),
                                  "offset": 0.0, "rate": 1.0,
                                  "audio": {"gain": track["gain"], "fadeIn": 0.6,
                                            "fadeOut": 1.2}}]})

    # 开场红字：黑底 + 毛笔红字（灰重影 + 浅色描边），整条时间线后移
    intro = cfg.get("intro") or {}
    seconds = float(intro.get("seconds", 0))
    project_styles = [SUB_STYLE, LABEL_STYLE, TITLE_STYLE]
    if seconds > 0:
        for track in tracks:
            if track["kind"] == "audio" and track["name"].startswith("BGM"):
                continue
            for clip in track["clips"]:
                clip["start"] = round(clip["start"] + seconds, 3)
        previous = duration
        duration = round(duration + seconds, 2)
        # BGM 不整体后移，但最后一段要铺到新的结尾，否则混音比画面短
        for track in tracks:
            if track["kind"] != "audio" or not track["name"].startswith("BGM"):
                continue
            for clip in track["clips"]:
                if abs(clip["start"] + clip["duration"] - previous) < 0.1:
                    clip["duration"] = round(duration - clip["start"], 2)
        add_media("m_black", "结尾卡", files["black"])
        title_style = intro.get("style", "title-red-brush")
        if title_style == "title-red-brush":
            project_styles.append(RED_TITLE_STYLE)
        # 开场静音：让混音的第一条音频从 0 开始，避免导出按 -shortest 截短画面
        silence = os.path.join(media_dir, "intro-silence.wav")
        with wave.open(silence, "wb") as handle:
            handle.setnchannels(1)
            handle.setsampwidth(2)
            handle.setframerate(24000)
            handle.writeframes(b"\0\0" * int(24000 * seconds))
        add_media("m_silence", "开场静音", silence, "audio", 0, 0, seconds, True)
        voice_track = next(track for track in tracks if track["id"] == "a1")
        voice_track["clips"].insert(0, {
            "id": "intro-silence", "mediaId": "m_silence", "start": 0.0,
            "duration": round(seconds, 3), "offset": 0.0, "rate": 1.0,
            "audio": {"gain": 1.0, "fadeIn": 0.0, "fadeOut": 0.0}})
        cards_track = next(track for track in tracks if track["id"] == "v2")
        cards_track["clips"].insert(0, {
            "id": "intro-card", "mediaId": "m_black", "start": 0.0,
            "duration": round(seconds, 3), "offset": 0.0, "rate": 1.0,
            "intro": {"kind": "fade", "duration": 0.12}})
        title_track = next(track for track in tracks if track["id"] == "t3")
        title_track["clips"].insert(0, {
            "id": "intro-title", "start": 0.08, "duration": round(seconds - 0.08, 3),
            "transform": {"x": 0.0, "y": -0.02, "scale": 1.0, "scaleX": 1.0,
                          "scaleY": 1.0, "opacity": 1.0},
            "text": {"text": intro.get("text", cfg["title"]),
                     "styleId": title_style,
                     "layout": {"fontSize": float(intro.get("size", 0.15)),
                                "letterSpacing": 0.04, "lineHeight": 1.2,
                                "align": "center"},
                     "animation": {"intro": {"kind": "perChar",
                                             "duration": round(seconds * 0.72, 2),
                                             "charDuration": 0.12}}}})

    for track in tracks:
        trim(track["clips"])
    project = {"version": 1, "fps": 30, "playhead": 0.0, "width": 1920, "height": 1080,
               "duration": duration, "media": media, "tracks": tracks,
               "textStyles": project_styles}
    with open(os.path.join(folder, "project.json"), "w", encoding="utf-8") as handle:
        json.dump(project, handle, ensure_ascii=False, indent=2)
    print(f"project -> data/{cfg['project']}  clips={sum(len(t['clips']) for t in tracks)}")


def stage_export(cfg):
    output = os.path.join(PROD, f"{cfg['project']}.mp4")
    command = ["cargo", "run", "-q", "-p", "todo_app", "--example", "export_project",
               "--", cfg["project"], output]
    print("export ->", output)
    subprocess.run(command, cwd=ROOT, check=True)


def stage_describe(cfg):
    """打印工程里每个元素的像素矩形，供人或 agent 核版式。"""
    output = subprocess.run(
        ["cargo", "run", "-q", "-p", "todo_app", "--example", "describe_project",
         "--", cfg["project"]],
        cwd=ROOT, check=True, capture_output=True, text=True, encoding="utf-8")
    view = json.loads(output.stdout)["document"]
    canvas = view["canvas"]
    print(f"canvas {canvas['width']}x{canvas['height']} @{canvas['fps']}fps  "
          f"elements {len(view['elements'])}")
    for element in view["elements"]:
        rect = element.get("rect")
        if not rect:
            continue
        print(f"  z={element['z']:<2} {element['kind']:<6} {element['name'][:16]:<18} "
              f"{element['start']:6.2f}+{element['duration']:5.2f}s  "
              f"({rect['x']:6.0f},{rect['y']:6.0f}) {rect['width']:5.0f}x{rect['height']:4.0f}px"
              + ("  ⚠ " + ", ".join(element["warnings"]) if element.get("warnings") else ""))


def stage_inspect(cfg):
    """渲染节目预览帧并降采样，供大模型自己看画面（审图闭环的输入端）。"""
    times = [float(value) for value in (cfg.get("inspect_times") or [])]
    if not times:
        plan = json.load(open(PLAN, encoding="utf-8"))
        sentences, duration = load_sentences()
        times = [round(sentences[shot["sentences"][0]]["start"] + 0.6, 2)
                 for shot in plan["shots"]][:8]
        if not times:
            times = [0.5, duration / 2, max(0.5, duration - 0.5)]
    out = os.path.join(PROD, "review")
    command = ["cargo", "run", "-q", "-p", "todo_app", "--example", "render_frame",
               "--", cfg["project"], out, *[f"{time:g}" for time in times]]
    result = subprocess.run(command, cwd=ROOT, check=True, capture_output=True,
                            text=True, encoding="utf-8")
    shots = [json.loads(line) for line in result.stdout.splitlines() if line.startswith("{")]
    width = int(cfg.get("inspect_width", 960))
    for item in shots:
        image = Image.open(item["png"]).convert("RGB")
        scale = width / image.size[0]
        small = image.resize((width, int(image.size[1] * scale)), Image.LANCZOS)
        target = os.path.splitext(item["png"])[0] + ".jpg"
        small.save(target, quality=88, optimize=True)
        print(f"{item['time']:6.2f}s -> {target}")
    print(f"{len(shots)} frames; 让模型逐张看这些 jpg 再决定改哪里")


FFMPEG = os.path.join(ROOT, "dependencies", "ffmpeg", "bin", "ffmpeg.exe")


def _project_doc(cfg):
    path = os.path.join(ROOT, "data", cfg["project"], "project.json")
    return json.load(open(path, encoding="utf-8"))


def _rects(project):
    """用与 Rust 侧一致的算法算出每个元素的像素矩形（图片按 aspect-fit）。"""
    canvas_w, canvas_h = project["width"], project["height"]
    media = {item["id"]: item for item in project["media"]}
    out = []
    for track in project["tracks"]:
        for clip in track["clips"]:
            source = clip.get("source") or {}
            if "mediaId" in clip:
                item = media.get(clip["mediaId"], {})
                width, height = item.get("width") or 0, item.get("height") or 0
                kind = item.get("kind") or "video"
                if kind == "audio" or not width or not height:
                    continue
                aspect = width / height
                canvas_aspect = canvas_w / canvas_h
                base_w = min(aspect / canvas_aspect, 1.0)
                base_h = min(canvas_aspect / aspect, 1.0)
                name = item.get("name", clip["mediaId"])
            elif "text" in clip:
                layout = clip["text"]["layout"]
                font = layout["fontSize"] * canvas_h
                chars = len(clip["text"]["text"])
                base_w = min(chars * font, canvas_w) / canvas_w
                base_h = min(font * layout["lineHeight"], canvas_h) / canvas_h
                kind, name = "text", clip["text"]["text"]
            else:
                continue
            transform = clip.get("transform") or {}
            scale = transform.get("scale", 1.0)
            width = base_w * scale * transform.get("scaleX", 1.0)
            height = base_h * scale * transform.get("scaleY", 1.0)
            cx = 0.5 + transform.get("x", 0.0)
            cy = 0.5 + transform.get("y", 0.0)
            out.append({
                "clip": clip["id"], "track": track["id"], "kind": kind, "name": name,
                "start": clip["start"], "duration": clip["duration"],
                "x": (cx - width / 2) * canvas_w, "y": (cy - height / 2) * canvas_h,
                "w": width * canvas_w, "h": height * canvas_h,
                "opacity": transform.get("opacity", 1.0),
            })
    return out


def _rubric(project):
    """版式与时序检查项：只报告，不修改。"""
    canvas_w, canvas_h = project["width"], project["height"]
    elements = _rects(project)
    findings = []

    def add(ident, level, title, detail):
        findings.append({"id": ident, "level": level, "title": title, "detail": detail})

    # 1. 文字压主体（字幕在底栏，属于设计，不算问题）
    pictures = [e for e in elements if e["kind"] in ("image", "video")]
    texts = [e for e in elements if e["kind"] == "text"]
    subtitle_zone = canvas_h * 0.78
    for text in texts:
        if text["y"] + text["h"] / 2 >= subtitle_zone:
            continue
        for picture in pictures:
            # 黑卡与底板上的大字是设计，不算压主体
            if picture["name"].startswith("结尾") or picture["name"] in ("背景板", "plate"):
                continue
            # 必须时间上真的同屏，否则开场红字会被算到第一镜头上
            if (text["start"] >= picture["start"] + picture["duration"]
                    or picture["start"] >= text["start"] + text["duration"]):
                continue
            ox = min(text["x"] + text["w"], picture["x"] + picture["w"]) - max(text["x"], picture["x"])
            oy = min(text["y"] + text["h"], picture["y"] + picture["h"]) - max(text["y"], picture["y"])
            if ox <= 0 or oy <= 0 or text["w"] * text["h"] == 0:
                continue
            ratio = (ox * oy) / (text["w"] * text["h"])
            if ratio > 0.35:
                add("text-on-subject", "P1", "标注压在画面上",
                    f"「{text['name'][:14]}」有 {ratio:.0%} 盖在「{picture['name'][:16]}」上"
                    f"（t={text['start']:.1f}s，字框 {text['x']:.0f},{text['y']:.0f} {text['w']:.0f}x{text['h']:.0f}）")
                break

    # 2. 出画（满幅铺底是设计，允许 5% 出血；超出才算问题）
    for element in elements:
        if element["kind"] == "text":
            continue
        if (element["x"] < -canvas_w * 0.05 or element["y"] < -canvas_h * 0.05
                or element["x"] + element["w"] > canvas_w * 1.05
                or element["y"] + element["h"] > canvas_h * 1.05):
            add("out-of-frame", "P2", "元素出血",
                f"「{element['name'][:16]}」超出画布（{element['x']:.0f},{element['y']:.0f}）"
                f" {element['w']:.0f}x{element['h']:.0f}")

    # 3. 单侧留白
    for picture in pictures:
        if picture["w"] * picture["h"] < canvas_w * canvas_h * 0.18:
            continue
        left = picture["x"]
        right = canvas_w - (picture["x"] + picture["w"])
        if abs(left - right) > canvas_w * 0.12:
            side = "左" if left > right else "右"
            add("side-void", "P1", "画面单侧留白过大",
                f"t={picture['start']:.1f}s「{picture['name'][:16]}」{side}侧空 {max(left, right) / canvas_w:.0%}")

    # 4. 同屏元素过多 / 字数过小
    for moment in [i * 0.5 for i in range(int(project["duration"] * 2))]:
        visible = [e for e in elements if e["start"] <= moment < e["start"] + e["duration"]]
        if len(visible) > 6:
            add("too-many", "P2", "同屏元素过多", f"t={moment:.1f}s 共 {len(visible)} 层")
            break
    for text in texts:
        font = text["h"] / max(1, text["name"].count("\n") + 1)
        if font < canvas_h * 0.035:
            add("small-text", "P2", "字号偏小", f"「{text['name'][:14]}」约 {font:.0f}px")

    # 5. 时序：镜头过短 / 切点过密
    shots = sorted(
        [clip for track in project["tracks"] if track["kind"] == "video"
         for clip in track["clips"] if clip.get("mediaId") in
         {item["id"] for item in project["media"] if (item.get("kind") or "") != "audio"}],
        key=lambda clip: clip["start"])
    shots = [clip for clip in shots if clip["duration"] > 0.3 and not clip["id"].startswith("intro")]
    for clip in shots:
        if clip["duration"] < 2.5:
            add("short-shot", "P2", "镜头过短", f"{clip['start']:.1f}s 起仅 {clip['duration']:.1f}s")
    cuts = sorted({round(clip["start"], 3) for clip in shots})[1:]
    for previous, current in zip(cuts, cuts[1:]):
        if current - previous < 1.5:
            add("cut-gap", "P2", "切点过密", f"{previous:.1f}s → {current:.1f}s 间隔 {current - previous:.2f}s")

    # 6. 字幕断续
    subtitles = sorted([e for e in text_blocks(project) if e["duration"] < 8], key=lambda e: e["start"])
    for before, after in zip(subtitles, subtitles[1:]):
        gap = after["start"] - (before["start"] + before["duration"])
        if gap > 1.2:
            add("subtitle-gap", "P2", "字幕空档过长",
                f"{before['start'] + before['duration']:.1f}s → {after['start']:.1f}s 空 {gap:.2f}s")
    return findings


def text_blocks(project):
    out = []
    for track in project["tracks"]:
        if track["kind"] != "text":
            continue
        for clip in track["clips"]:
            out.append({"start": clip["start"], "duration": clip["duration"],
                        "name": clip["text"]["text"]})
    return out


def stage_review(cfg):
    """出小 mp4 + 切点帧序列 + rubric 报告；只给模型看，不自动改。"""
    project = _project_doc(cfg)
    review = os.path.join(PROD, "review")
    os.makedirs(review, exist_ok=True)
    findings = _rubric(project)

    video = os.path.join(PROD, f"{cfg['project']}.mp4")
    preview = os.path.join(review, "preview-480p.mp4")
    cuts = sorted({round(clip["start"], 3)
                   for track in project["tracks"] if track["kind"] == "video"
                   for clip in track["clips"] if clip["duration"] > 0.5})[1:]
    frames = []
    if os.path.exists(video):
        subprocess.run([FFMPEG, "-hide_banner", "-loglevel", "error", "-y", "-i", video,
                        "-vf", "scale=480:-1", "-c:v", "libx264", "-crf", "28",
                        "-preset", "veryfast", "-an", preview], check=True)
        for index, cut in enumerate(cuts):
            for offset in (-0.4, -0.15, 0.0, 0.15, 0.4):
                moment = max(0.0, cut + offset)
                path = os.path.join(review, f"cut{index:02d}_{offset:+.2f}.jpg")
                subprocess.run([FFMPEG, "-hide_banner", "-loglevel", "error", "-y",
                                "-ss", f"{moment:.3f}", "-i", video, "-frames:v", "1",
                                "-vf", "scale=480:-1", path], check=True)
                frames.append({"cut": cut, "offset": offset, "path": path})
    sheet = build_review_sheet(frames, review) if frames else None

    report = {"project": cfg["project"], "findings": findings,
              "preview": preview if os.path.exists(preview) else None,
              "sheet": sheet, "frames": len(frames)}
    with open(os.path.join(review, "report.json"), "w", encoding="utf-8") as handle:
        json.dump(report, handle, ensure_ascii=False, indent=1)
    lines = [f"# {cfg['project']} 审片报告", "",
             f"- 小 mp4：`{preview}`" if os.path.exists(preview) else "- 小 mp4：未导出",
             f"- 切点帧：{len(frames)} 张，拼图 `{sheet}`" if sheet else "- 切点帧：无",
             "", "## 检查项", ""]
    for finding in findings:
        lines.append(f"- **{finding['level']} {finding['title']}** — {finding['detail']}")
    if not findings:
        lines.append("- 无硬性问题（观感仍需模型判断）")
    with open(os.path.join(review, "report.md"), "w", encoding="utf-8") as handle:
        handle.write("\n".join(lines) + "\n")
    print(f"findings {len(findings)}; frames {len(frames)}; preview {os.path.exists(preview)}")
    for finding in findings:
        print(f"  {finding['level']} {finding['title']}: {finding['detail']}")


def build_review_sheet(frames, review):
    """把每个切点前后 5 帧拼成一张图，方便一次看完所有转场。"""
    if not frames:
        return None
    per_cut = 5
    rows = (len(frames) + per_cut - 1) // per_cut
    tile_w, tile_h = 320, 180
    sheet = Image.new("RGB", (tile_w * per_cut, tile_h * rows), (16, 16, 18))
    for index, frame in enumerate(frames):
        image = Image.open(frame["path"]).convert("RGB").resize((tile_w, tile_h), Image.LANCZOS)
        sheet.paste(image, ((index % per_cut) * tile_w, (index // per_cut) * tile_h))
    path = os.path.join(review, "cuts-sheet.jpg")
    sheet.save(path, quality=86, optimize=True)
    return path


STAGES = {"script": stage_script, "tts": stage_tts, "plan": stage_plan,
          "scenes": stage_scenes, "project": stage_project, "describe": stage_describe,
          "inspect": stage_inspect, "review": stage_review, "export": stage_export}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stage", default="all", choices=["all", *STAGES])
    args = parser.parse_args()
    cfg = config()
    names = [args.stage] if args.stage != "all" else list(STAGES)
    for name in names:
        print(f"== {name} ==")
        STAGES[name](cfg)


if __name__ == "__main__":
    sys.exit(main())
