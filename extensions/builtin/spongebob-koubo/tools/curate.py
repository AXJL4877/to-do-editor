"""Turn raw generated pictures into library assets and a contact sheet.

    python tools/curate.py <raw dir> [--sheet out.jpg]

Cutouts are trimmed to their alpha box (small margin) and scaled to at most
1024 px; the plate is cropped to 16:9, scaled to 1920x1080 and softened so
the foreground reads like the reference. `library.json` is rebuilt from
`tools/library-meta.json` (names, descriptions) for the files that exist.
"""
import json
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

PACKAGE = Path(__file__).resolve().parent.parent
OUT = PACKAGE / "assets" / "library"
META = json.loads((Path(__file__).parent / "library-meta.json").read_text(encoding="utf-8"))


def cutout(src, dst):
    image = Image.open(src).convert("RGBA")
    alpha = image.getchannel("A").point(lambda a: 255 if a > 12 else 0)
    box = alpha.getbbox()
    if box:
        pad = 12
        box = (max(0, box[0] - pad), max(0, box[1] - pad),
               min(image.width, box[2] + pad), min(image.height, box[3] + pad))
        image = image.crop(box)
    image.thumbnail((1024, 1024), Image.LANCZOS)
    image.save(dst, optimize=True)
    return image.size


def plate(src, dst, blur):
    image = Image.open(src).convert("RGB")
    w, h = image.size
    target_h = int(w * 9 / 16)
    top = max(0, (h - target_h) // 2)
    image = image.crop((0, top, w, top + target_h)).resize((1920, 1080), Image.LANCZOS)
    if blur:
        image = image.filter(ImageFilter.GaussianBlur(blur))
    image.save(dst, quality=92)
    return image.size


def sheet(entries, path):
    cell = 220
    cols = 6
    rows = (len(entries) + cols - 1) // cols
    canvas = Image.new("RGB", (cols * cell, rows * (cell + 24)), (40, 60, 70))
    draw = ImageDraw.Draw(canvas)
    for index, entry in enumerate(entries):
        image = Image.open(OUT / entry["file"]).convert("RGBA")
        image.thumbnail((cell - 10, cell - 10))
        x, y = (index % cols) * cell, (index // cols) * (cell + 24)
        canvas.paste(image, (x + (cell - image.width) // 2, y + (cell - image.height) // 2), image)
        draw.text((x + 6, y + cell + 4), entry["id"], fill=(255, 255, 255))
    canvas.save(path, quality=88)


def main():
    raw = Path(sys.argv[1])
    OUT.mkdir(parents=True, exist_ok=True)
    entries = []
    for meta in META:
        files = sorted(raw.glob(meta["id"] + ".*"))
        if not files:
            continue
        if meta["kind"] == "plate":
            name = meta["id"] + ".jpg"
            size = plate(files[0], OUT / name, meta.get("blur", 6))
        else:
            name = meta["id"] + ".png"
            size = cutout(files[0], OUT / name)
        entries.append({**meta, "file": name, "width": size[0], "height": size[1]})
    (OUT / "library.json").write_text(json.dumps(entries, ensure_ascii=False, indent=1), encoding="utf-8")
    if "--sheet" in sys.argv:
        sheet(entries, sys.argv[sys.argv.index("--sheet") + 1])
    print(f"{len(entries)} assets")


if __name__ == "__main__":
    main()
