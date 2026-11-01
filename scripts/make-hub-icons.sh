#!/usr/bin/env bash
# Makes Xenify Hub's icons (crates/xenify-hub/icons): a rounded #C2410C
# square with a white X set in Figtree (OFL-1.1). Needs ImageMagick.
set -euo pipefail
cd "$(dirname "$0")/../crates/xenify-hub"
gen() { # size out [margin %: tray.png keeps clear of its neighbours in a system tray]
  local s=$1 out=$2 m=${3:-0} stroke=$(( $1 * 4 / 100 > 1 ? $1 * 4 / 100 : 1 ))
  local o=$(( s * m / 100 )) e=$(( s - 1 - s * m / 100 ))
  convert -size "${s}x${s}" xc:none -fill '#C2410C' \
    -draw "roundrectangle $o,$o,$e,$e,$(( (e - o) * 22 / 100 )),$(( (e - o) * 22 / 100 ))" \
    -font ui/fonts/Figtree.ttf -fill white -stroke white -strokewidth "$stroke" \
    -pointsize $(( (s - 2 * s * m / 100) * 56 / 100 )) -gravity center -annotate +0+$((s * 2 / 100)) X "PNG32:$out"
}
gen 512 icons/icon.png
gen 32 icons/32x32.png
gen 128 icons/128x128.png
gen 256 icons/128x128@2x.png
gen 64 icons/tray.png 9
# The Windows and Mac icon files, from the big one (cargo tauri icon writes
# a whole set; only these two are kept).
tmp=$(mktemp -d)
cargo tauri icon icons/icon.png -o "$tmp" >/dev/null
cp "$tmp/icon.ico" "$tmp/icon.icns" icons/
rm -rf "$tmp"
