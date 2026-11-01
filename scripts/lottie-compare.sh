#!/bin/bash
# Renders frames of a project with Xenify Draw and with lottie-web (headless Chromium)
# side by side, to check the Lottie export. Needs `chromium` (apt install chromium),
# ImageMagick and internet access (lottie-web from the CDN).
# usage: scripts/lottie-compare.sh project.xenify out_dir frame...
set -e
P=$1; OUT=$2; shift 2
mkdir -p "$OUT"
cargo build -q --release -p xenify-cli
CLI=target/release/xenify-cli
$CLI render "$P" -o "$OUT/anim.html" 2>/dev/null
W=$(python3 -c "import json;print(json.load(open('$P'))['compositions'][0]['width'])")
H=$(python3 -c "import json;print(json.load(open('$P'))['compositions'][0]['height'])")
SC=$(python3 -c "print(960/$W)"); WH=$(python3 -c "print(round($H*960/$W))")
rm -rf "$OUT/frames"
$CLI render "$P" -o "$OUT/frames" --format png --scale "$SC" >/dev/null 2>&1
rows=()
for f in "$@"; do
  chromium --headless --no-sandbox --disable-gpu --hide-scrollbars --window-size=960,$WH \
    --virtual-time-budget=4000 --screenshot="$OUT/lottie_$f.png" "file://$(realpath "$OUT")/anim.html#frame=$f" >/dev/null 2>&1
  printf -v ff "%05d" $((f + 1)) # ffmpeg numbers PNG frames from 1
  convert "$OUT/frames/frame_$ff.png" "$OUT/lottie_$f.png" +append "$OUT/row_$f.png"
  rows+=("$OUT/row_$f.png")
done
convert "${rows[@]}" -append "$OUT/compare.png"
echo "$OUT/compare.png  (left: Xenify Draw, right: lottie-web)"
