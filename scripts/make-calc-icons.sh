#!/usr/bin/env bash
# Makes Xenify Calc's icons (crates/xenify-calc-app/icons): a rounded #C2410C
# square with a white C set in Figtree (OFL-1.1). Needs ImageMagick.
set -euo pipefail
cd "$(dirname "$0")/../crates/xenify-calc-app"
gen() {
  local s=$1 out=$2 stroke=$(( $1 * 4 / 100 > 1 ? $1 * 4 / 100 : 1 ))
  convert -size "${s}x${s}" xc:none -fill '#C2410C' \
    -draw "roundrectangle 0,0,$((s - 1)),$((s - 1)),$((s * 22 / 100)),$((s * 22 / 100))" \
    -font ui/fonts/Figtree.ttf -fill white -stroke white -strokewidth "$stroke" \
    -pointsize $((s * 56 / 100)) -gravity center -annotate +0+$((s * 2 / 100)) C "PNG32:$out"
}
gen 512 icons/icon.png
gen 32 icons/32x32.png
gen 128 icons/128x128.png
gen 256 icons/128x128@2x.png
# The Windows and Mac icon files, from the big one (cargo tauri icon writes
# a whole set; only these two are kept).
tmp=$(mktemp -d)
cargo tauri icon icons/icon.png -o "$tmp" >/dev/null
cp "$tmp/icon.ico" "$tmp/icon.icns" icons/
rm -rf "$tmp"
