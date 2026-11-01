#!/usr/bin/env bash
# Packages Xenify Plan (crates/xenify-plan-app) into dist-plan/. Spelling and
# grammar come from Write's checker, built into the program (xenify-write-host);
# the mistake spotter model is used when Write has installed it, not bundled.
#
#   scripts/package-plan.sh            for this computer's system
#   scripts/package-plan.sh linux      .deb, .rpm, .AppImage and a .tar.gz
#   scripts/package-plan.sh windows    setup .exe (NSIS); builds on Windows, or on Linux
#                                       with mingw-w64 and nsis installed
#   scripts/package-plan.sh macos      .dmg and .app for Apple silicon and Intel; needs a Mac
#
# The browser version is scripts/build-plan-web.sh. Signing, and where each
# system's packages are built, are as for Write: docs/write-packaging.md.
set -euo pipefail
cd "$(dirname "$0")/.."
app=crates/xenify-plan-app
out=dist-plan
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

echo "Packaging Xenify Plan $version for ${system}…"
# The engine as WebAssembly, which the app bundles with its screens (ui/pkg).
scripts/build-plan-web.sh
# Every system's icon sizes, made from icons/icon.png (scripts/make-plan-icons.sh).
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
    cp "$bundle"/deb/Xenify\ Plan*.deb "$bundle"/rpm/Xenify\ Plan*.rpm "$bundle"/appimage/Xenify\ Plan*.AppImage "$out/"
    # A plain folder too: unpack anywhere and run xenify-plan.
    stage="$out/xenify-plan-$version"
    rm -rf "$stage" && mkdir -p "$stage"
    cp target/release/xenify-plan "$stage/" 2>/dev/null || cp target/release/xenify-plan-app "$stage/xenify-plan"
    strip "$stage/xenify-plan" 2>/dev/null || true
    cp $app/packaging/xenify-plan.desktop "$stage/"
    cp $app/icons/icon.png "$stage/xenify-plan.png"
    cp LICENSE THIRD_PARTY_LICENSES.txt "$stage/"
    tar -C "$out" -czf "$out/xenify-plan-$version-$(uname -m)-linux.tar.gz" "xenify-plan-$version"
    rm -rf "$stage"
    ;;
windows)
    # On Linux this cross-builds with mingw-w64 (sudo apt install gcc-mingw-w64-x86-64
    # g++-mingw-w64-x86-64 nsis; rustup target add x86_64-pc-windows-gnu).
    target=x86_64-pc-windows-msvc
    [ "$(uname -s)" = Linux ] && target=x86_64-pc-windows-gnu
    (cd $app && cargo tauri build --target $target --bundles nsis)
    cp target/$target/release/bundle/nsis/Xenify\ Plan*.exe "$out/"
    ;;
macos)
    rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
    (cd $app && cargo tauri build --target universal-apple-darwin --bundles app,dmg)
    cp target/universal-apple-darwin/release/bundle/dmg/Xenify\ Plan*.dmg "$out/"
    ;;
*) echo "Unknown system $system; say linux, windows or macos" >&2; exit 1 ;;
esac
# File names without spaces ("Xenify Plan_0.1.0_amd64.deb" → xenify-plan_0.1.0_amd64.deb).
for f in "$out"/Xenify\ Plan*; do [ -e "$f" ] && mv "$f" "$out/$(basename "$f" | sed "s/^Xenify Plan/xenify-plan/")"; done
echo "Done:"; ls -lh "$out" | sed -n '2,$p'
