#!/usr/bin/env bash
# Makes Xenify Chat's icons (crates/xenify-chat-app/icons): a rounded blue
# (#1D4ED8, Chat's colour in the Hub) square with a white speech bubble and
# three dots, drawn here (no outside artwork). Also tray.png: the same
# picture with a clear margin, so it doesn't touch its neighbours in a
# system tray. Needs ImageMagick and the Tauri CLI.
set -euo pipefail
cd "$(dirname "$0")/../crates/xenify-chat-app"
draw() { # size out [margin %]
  local s=$1 out=$2 m=${3:-0}
  local o=$(( s * m / 100 )) e=$(( s - 1 - s * m / 100 ))
  local w=$(( e - o ))
  local r=$(( w * 22 / 100 ))
  # The bubble: a rounded box with a tail at its lower left.
  local bx0=$(( o + w * 20 / 100 )) by0=$(( o + w * 24 / 100 )) bx1=$(( o + w * 80 / 100 )) by1=$(( o + w * 66 / 100 ))
  local br=$(( w * 12 / 100 ))
  local t1x=$(( o + w * 30 / 100 )) t2x=$(( o + w * 46 / 100 )) tty=$(( o + w * 80 / 100 )) tx=$(( o + w * 27 / 100 ))
  local dy=$(( (by0 + by1) / 2 )) dr=$(( w * 45 / 1000 > 1 ? w * 45 / 1000 : 1 ))
  local d1=$(( o + w * 37 / 100 )) d2=$(( o + w * 50 / 100 )) d3=$(( o + w * 63 / 100 ))
  convert -size "${s}x${s}" xc:none -fill '#1D4ED8' \
    -draw "roundrectangle $o,$o,$e,$e,$r,$r" \
    -fill white -draw "roundrectangle $bx0,$by0,$bx1,$by1,$br,$br" \
    -draw "polygon $t1x,$((by1 - 2)) $t2x,$((by1 - 2)) $tx,$tty" \
    -fill '#1D4ED8' -draw "circle $d1,$dy $((d1 + dr)),$dy" -draw "circle $d2,$dy $((d2 + dr)),$dy" -draw "circle $d3,$dy $((d3 + dr)),$dy" \
    "PNG32:$out"
}
draw 512 icons/icon.png
draw 32 icons/32x32.png
draw 128 icons/128x128.png
draw 256 icons/128x128@2x.png
draw 64 icons/tray.png 9
tmp=$(mktemp -d)
cargo tauri icon icons/icon.png -o "$tmp" >/dev/null
cp "$tmp/icon.ico" "$tmp/icon.icns" icons/
rm -rf "$tmp"
