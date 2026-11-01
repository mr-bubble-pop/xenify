#!/usr/bin/env bash
# Packages Xenify Calc (crates/xenify-calc-app) into dist-calc/. Calc has no
# spelling checker, so there's no model or background checker to build in.
#
#   scripts/package-calc.sh            for this computer's system
#   scripts/package-calc.sh linux      .deb, .rpm, .AppImage and a .tar.gz
#   scripts/package-calc.sh windows    setup .exe (NSIS); builds on Windows, or on Linux
#                                       with mingw-w64 and nsis installed
#   scripts/package-calc.sh macos      .dmg and .app for Apple silicon and Intel; needs a Mac
#
# The browser version is scripts/build-calc-web.sh. Signing, and where each
# system's packages are built, are as for Write: docs/write-packaging.md.
set -euo pipefail
cd "$(dirname "$0")/.."
app=crates/xenify-calc-app
out=dist-calc
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
system=${1:-}
if [ -z "$system" ]; then
    case "$(uname -s)" in
        Linux) system=linux ;;
        Darwin) system=macos ;;
        MINGW*|MSYS*|CYGWIN*) system=windows ;;
        *) echo "Unknown system; say linux, windows or macos" >&2; exit 1 ;;
    esac
fi
command -v cargo-tauri >/dev/null || { echo "Needs Tauri's CLI: cargo install tauri-cli --version '^2' --locked" >&2; exit 1; }

echo "Packaging Xenify Calc $version for ${system}…"
# The engine as WebAssembly, which the app bundles with its screens (ui/pkg).
scripts/build-calc-web.sh
# Every system's icon sizes, made from icons/icon.png (scripts/make-calc-icons.sh).
if [ ! -f $app/icons/icon.ico ] || [ ! -f $app/icons/icon.icns ]; then
    # (Made in a folder of their own: the tool also rewrites icon.png.)
    tmp=$(mktemp -d)
    cargo tauri icon $app/icons/icon.png -o "$tmp" >/dev/null
    cp "$tmp"/{32x32.png,128x128.png,128x128@2x.png,icon.ico,icon.icns} $app/icons/
    rm -rf "$tmp"
fi
# Licences of everything built in.
if command -v cargo-about >/dev/null; then scripts/licenses.sh; fi
mkdir -p "$out"

case $system in
linux)
    (cd $app && cargo tauri build --bundles deb,rpm,appimage)
    bundle=target/release/bundle
    cp "$bundle"/deb/Xenify\ Calc*.deb "$bundle"/rpm/Xenify\ Calc*.rpm "$bundle"/appimage/Xenify\ Calc*.AppImage "$out/"
    # A plain folder too: unpack anywhere and run xenify-calc.
    stage="$out/xenify-calc-$version"
    rm -rf "$stage" && mkdir -p "$stage"
    cp target/release/xenify-calc "$stage/" 2>/dev/null || cp target/release/xenify-calc-app "$stage/xenify-calc"
    strip "$stage/xenify-calc" 2>/dev/null || true
    cp $app/packaging/xenify-calc.desktop "$stage/"
    cp $app/icons/icon.png "$stage/xenify-calc.png"
    cp LICENSE THIRD_PARTY_LICENSES.txt "$stage/"
    tar -C "$out" -czf "$out/xenify-calc-$version-$(uname -m)-linux.tar.gz" "xenify-calc-$version"
    rm -rf "$stage"
    ;;
windows)
    # On Linux this cross-builds with mingw-w64 (sudo apt install gcc-mingw-w64-x86-64
    # g++-mingw-w64-x86-64 nsis; rustup target add x86_64-pc-windows-gnu).
    target=x86_64-pc-windows-msvc
    [ "$(uname -s)" = Linux ] && target=x86_64-pc-windows-gnu
    (cd $app && cargo tauri build --target $target --bundles nsis)
    cp target/$target/release/bundle/nsis/Xenify\ Calc*.exe "$out/"
    ;;
macos)
    rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
    (cd $app && cargo tauri build --target universal-apple-darwin --bundles app,dmg)
    cp target/universal-apple-darwin/release/bundle/dmg/Xenify\ Calc*.dmg "$out/"
    ;;
*) echo "Unknown system $system; say linux, windows or macos" >&2; exit 1 ;;
esac
# File names without spaces ("Xenify Calc_0.1.0_amd64.deb" → xenify-calc_0.1.0_amd64.deb).
for f in "$out"/Xenify\ Calc*; do [ -e "$f" ] && mv "$f" "$out/$(basename "$f" | sed "s/^Xenify Calc/xenify-calc/")"; done
echo "Done:"; ls -lh "$out" | sed -n '2,$p'
