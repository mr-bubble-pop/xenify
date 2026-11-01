#!/usr/bin/env python3
"""Builds the site's tour film from chunks, one per section.

Each chunk is a small JSON file in scripts/tour/chunks/ (its name orders it:
00-intro.json, 10-wiki.json…) listing its shots:

    {"card": {"title": "…", "sub": "…"}, "seconds": 3}
    {"clip": "site/media/calc-lookup.mp4", "from": 0.4, "to": 9.5, "speed": 1.2, "caption": "…"}
    {"still": "site/media/chat-mail.webp", "seconds": 5, "caption": "…"}

A clip or still can add "crop": [x, y, w, h] (in its own pixels) to show
just part of it, larger.

Every chunk becomes its own film in target/tour/ (1280x720, 30 fps, H.264,
all with the same settings). A chunk is only made again when its JSON, a
picture or film it uses, or this script changes, so changing one section
costs one section. The films are then joined without encoding again into
site/media/tour.mp4 (and tour.webm, made from that), with a poster
picture site/media/tour.webp from the first chunk.

    python3 scripts/tour/build.py            # what changed, then the whole film
    python3 scripts/tour/build.py 30-calc    # just that chunk (and the whole film)
    python3 scripts/tour/build.py --list     # chunks and their lengths

Needs ffmpeg with libfreetype (drawtext). Text uses the site's Figtree font.
"""
import hashlib
import json
import os
import subprocess
import sys
import tempfile

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
CHUNKS = os.path.join(ROOT, "scripts", "tour", "chunks")
OUT = os.path.join(ROOT, "target", "tour")
FONT = os.path.join(ROOT, "site", "fonts", "Figtree.ttf")
W, H, FPS = 1280, 720, 30
# The site's colours.
PAPER, INK, MUTED, ACCENT = "0xF6F3EF", "0x1C1917", "0x57534E", "0xC2410C"
# Where pictures and films sit: a box at the top, the caption under it.
BOX_W, BOX_H, BOX_Y = 1160, 560, 28
CAPTION_Y = 618
ENCODE = ["-c:v", "libx264", "-preset", "medium", "-crf", "20", "-pix_fmt", "yuv420p", "-r", str(FPS),
          "-video_track_timescale", "15360", "-an"]


def run(args):
    r = subprocess.run(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if r.returncode:
        sys.exit(f"ffmpeg failed:\n{' '.join(args)}\n{r.stderr[-3000:]}")
    return r.stdout


def text_file(tmp, text):
    """drawtext reads its text from a file (no escaping to get wrong)."""
    fd, path = tempfile.mkstemp(dir=tmp, suffix=".txt")
    with os.fdopen(fd, "w") as f:
        f.write(text)
    return path.replace(":", r"\:")


def drawtext(tmp, text, size, colour, y, extra=""):
    return (f"drawtext=fontfile={FONT}:textfile={text_file(tmp, text)}:fontsize={size}:fontcolor={colour}"
            f":x=(w-text_w)/2:y={y}:line_spacing={size // 4}{extra}")


def caption_filter(tmp, shot):
    c = shot.get("caption", "")
    return "," + drawtext(tmp, c, 34, INK, CAPTION_Y) if c else ""


def fade(seconds):
    """A short fade in and out, so shots don't jump."""
    f = min(0.35, seconds / 6)
    return f",fade=t=in:st=0:d={f:.2f}:color={PAPER},fade=t=out:st={seconds - f:.2f}:d={f:.2f}:color={PAPER}"


def crop(shot):
    c = shot.get("crop")
    return f"crop={c[2]}:{c[3]}:{c[0]}:{c[1]}," if c else ""


def shot_film(tmp, shot, n):
    out = os.path.join(tmp, f"shot{n:02d}.mp4")
    if "card" in shot:
        secs = float(shot.get("seconds", 3))
        c = shot["card"]
        title, sub = c.get("title", ""), c.get("sub", "")
        lines = title.count("\n") + 1
        ty = (H - lines * 78 - (70 if sub else 0)) // 2
        vf = drawtext(tmp, title, 66, INK, ty)
        if sub:
            vf += "," + drawtext(tmp, sub, 32, ACCENT if c.get("accent") else MUTED, ty + lines * 82 + 18)
        if c.get("tag"):
            vf += "," + drawtext(tmp, c["tag"].upper(), 22, ACCENT, ty - 56)
        vf += fade(secs)
        run(["ffmpeg", "-y", "-f", "lavfi", "-i", f"color=c={PAPER}:s={W}x{H}:r={FPS}:d={secs}", "-vf", vf, "-t", str(secs), *ENCODE, out])
    elif "clip" in shot:
        src = os.path.join(ROOT, shot["clip"])
        start, end = float(shot.get("from", 0)), float(shot.get("to", 0)) or None
        speed = float(shot.get("speed", 1))
        length = ((end or probe(src)) - start) / speed
        fit = f"scale={BOX_W}:{BOX_H}:force_original_aspect_ratio=decrease"
        vf = (f"[0:v]setpts=(PTS-STARTPTS)/{speed},{crop(shot)}{fit},fps={FPS}[v];"
              f"color=c={PAPER}:s={W}x{H}:r={FPS}:d={length:.3f}[bg];"
              f"[bg][v]overlay=x=(W-w)/2:y={BOX_Y}:shortest=1{caption_filter(tmp, shot)}{fade(length)}")
        trim = ["-ss", str(start)] + (["-to", str(end)] if end else [])
        run(["ffmpeg", "-y", *trim, "-i", src, "-filter_complex", vf, "-t", f"{length:.3f}", *ENCODE, out])
    elif "still" in shot:
        src = os.path.join(ROOT, shot["still"])
        secs = float(shot.get("seconds", 5))
        # Still, fitted into the box (no zoom: a moving picture of a screen wobbles).
        fit = f"scale={BOX_W}:{BOX_H}:force_original_aspect_ratio=decrease:flags=lanczos"
        vf = (f"[0:v]{crop(shot)}{fit},fps={FPS}[v];"
              f"color=c={PAPER}:s={W}x{H}:r={FPS}:d={secs}[bg];"
              f"[bg][v]overlay=x=(W-w)/2:y={BOX_Y}:shortest=1{caption_filter(tmp, shot)}{fade(secs)}")
        run(["ffmpeg", "-y", "-loop", "1", "-i", src, "-filter_complex", vf, "-t", str(secs), *ENCODE, out])
    else:
        sys.exit(f"a shot needs card, clip or still: {shot}")
    return out


def probe(path):
    return float(run(["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", path]).strip())


def chunk_hash(name, spec):
    h = hashlib.sha256()
    h.update(open(__file__, "rb").read())
    h.update(json.dumps(spec, sort_keys=True).encode())
    for shot in spec["shots"]:
        for key in ("clip", "still"):
            if key in shot:
                h.update(open(os.path.join(ROOT, shot[key]), "rb").read())
    return h.hexdigest()


def build_chunk(name, spec, force=False):
    film = os.path.join(OUT, name + ".mp4")
    stamp = film + ".hash"
    want = chunk_hash(name, spec)
    if not force and os.path.exists(film) and os.path.exists(stamp) and open(stamp).read() == want:
        return film, False
    with tempfile.TemporaryDirectory() as tmp:
        shots = [shot_film(tmp, s, i) for i, s in enumerate(spec["shots"])]
        lst = os.path.join(tmp, "list.txt")
        with open(lst, "w") as f:
            f.writelines(f"file '{s}'\n" for s in shots)
        run(["ffmpeg", "-y", "-f", "concat", "-safe", "0", "-i", lst, "-c", "copy", film])
    with open(stamp, "w") as f:
        f.write(want)
    return film, True


def chunks():
    names = sorted(n[:-5] for n in os.listdir(CHUNKS) if n.endswith(".json"))
    return [(n, json.load(open(os.path.join(CHUNKS, n + ".json")))) for n in names]


def main():
    os.makedirs(OUT, exist_ok=True)
    args = sys.argv[1:]
    all_chunks = chunks()
    if "--list" in args:
        total = 0
        for name, spec in all_chunks:
            film = os.path.join(OUT, name + ".mp4")
            d = probe(film) if os.path.exists(film) else 0
            total += d
            print(f"{name:16} {d:6.1f} s  {spec.get('section', '')}")
        print(f"{'total':16} {total:6.1f} s")
        return
    films = []
    for name, spec in all_chunks:
        film, made = build_chunk(name, spec, force=name in args)
        print(("made " if made else "kept ") + name)
        films.append(film)
    media = os.path.join(ROOT, "site", "media")
    lst = os.path.join(OUT, "list.txt")
    with open(lst, "w") as f:
        f.writelines(f"file '{x}'\n" for x in films)
    mp4 = os.path.join(media, "tour.mp4")
    run(["ffmpeg", "-y", "-f", "concat", "-safe", "0", "-i", lst, "-c", "copy", "-movflags", "+faststart", mp4])
    run(["ffmpeg", "-y", "-i", mp4, "-c:v", "libvpx-vp9", "-b:v", "0", "-crf", "36", "-row-mt", "1", "-deadline", "good", "-cpu-used", "4", "-an",
         os.path.join(media, "tour.webm")])
    # The poster: a moment of the real apps (the second chunk's first picture),
    # not a title card that repeats the page's own heading.
    poster = films[1] if len(films) > 1 else films[0]
    run(["ffmpeg", "-y", "-ss", "4.5", "-i", poster, "-frames:v", "1", "-quality", "85", os.path.join(media, "tour.webp")])
    print(f"site/media/tour.mp4: {probe(mp4):.1f} s")


if __name__ == "__main__":
    main()
