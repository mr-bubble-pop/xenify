#!/usr/bin/env bash
# Packages Xenify Draw (crates/xenify-draw-app) into dist-draw/, with the
# spelling and grammar checker and its mistake-spotter model built in, so
# nothing else needs installing.
#
#   scripts/package-draw.sh            for this computer's system
#   scripts/package-draw.sh linux      .deb, .rpm, .AppImage and a .tar.gz
#   scripts/package-draw.sh windows    setup .exe (NSIS); builds on Windows, or on Linux
#                                       with mingw-w64 and nsis installed
#   scripts/package-draw.sh macos      .dmg and .app for Apple silicon and Intel; needs a Mac
#
# The browser version is scripts/build-draw-web.sh. Signing, and where each
# system's packages are built, are as for Write: docs/write-packaging.md.
set -euo pipefail
cd "$(dirname "$0")/.."
app=crates/xenify-draw-app
out=dist-draw
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

echo "Packaging Xenify Draw $version for ${system}…"
scripts/stage-spotter.sh
# The engine as WebAssembly, which the app bundles with its screens (ui/pkg).
scripts/build-draw-web.sh
# Every system's icon sizes, made from icons/icon.png.
if [ ! -f $app/icons/icon.ico ] || [ ! -f $app/icons/icon.icns ]; then
    # (Made in a folder of their own: the tool also rewrites icon.png.)
    tmp=$(mktemp -d)
    cargo tauri icon $app/icons/icon.png -o "$tmp" >/dev/null
    cp "$tmp"/{32x32.png,128x128.png,128x128@2x.png,icon.ico,icon.icns} $app/icons/
    rm -rf "$tmp"
fi
# Licences of everything built in.
if command -v cargo-about >/dev/null; then scripts/licenses.sh; fi
mkdir -p "$out" $app/binaries

# The shared background checker (xenify-checker) goes next to the app on
# Linux and macOS, so every Xenify app on the computer uses one copy of the
# model. Tauri wants it named after the target.
sidecar() { # target-triple [cargo target]
    cargo build --release -p xenify-checker ${2:+--target $2}
    cp "target/${2:+$2/}release/xenify-checker" "$app/binaries/xenify-checker-$1"
}

case $system in
linux)
    triple=$(rustc -vV | sed -n 's/^host: //p')
    sidecar "$triple"
    (cd $app && cargo tauri build --bundles deb,rpm,appimage)
    bundle=target/release/bundle
    cp "$bundle"/deb/Xenify\ Draw*.deb "$bundle"/rpm/Xenify\ Draw*.rpm "$bundle"/appimage/Xenify\ Draw*.AppImage "$out/"
    # A plain folder too: unpack anywhere and run xenify-draw.
    stage="$out/xenify-draw-$version"
    rm -rf "$stage" && mkdir -p "$stage/spotter"
    cp target/release/xenify-draw "$stage/" 2>/dev/null || cp target/release/xenify-draw-app "$stage/xenify-draw"
    cp target/release/xenify-checker "$stage/"
    strip "$stage/xenify-draw" "$stage/xenify-checker" 2>/dev/null || true
    cp target/spotter/config.json target/spotter/tokenizer.json target/spotter/model.f16.safetensors target/spotter/README.md target/spotter/LICENSE-DeBERTa.txt "$stage/spotter/"
    cp $app/packaging/xenify-draw.desktop "$stage/"
    cp $app/icons/icon.png "$stage/xenify-draw.png"
    cp LICENSE THIRD_PARTY_LICENSES.txt "$stage/"
    tar -C "$out" -czf "$out/xenify-draw-$version-$(uname -m)-linux.tar.gz" "xenify-draw-$version"
    rm -rf "$stage"
    ;;
windows)
    # On Linux this cross-builds with mingw-w64 (sudo apt install gcc-mingw-w64-x86-64
    # g++-mingw-w64-x86-64 nsis; rustup target add x86_64-pc-windows-gnu).
    target=x86_64-pc-windows-msvc
    [ "$(uname -s)" = Linux ] && target=x86_64-pc-windows-gnu
    (cd $app && cargo tauri build --target $target --bundles nsis)
    cp target/$target/release/bundle/nsis/Xenify\ Draw*.exe "$out/"
    ;;
macos)
    rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
    sidecar aarch64-apple-darwin aarch64-apple-darwin
    sidecar x86_64-apple-darwin x86_64-apple-darwin
    lipo -create -output $app/binaries/xenify-checker-universal-apple-darwin \
        $app/binaries/xenify-checker-aarch64-apple-darwin $app/binaries/xenify-checker-x86_64-apple-darwin
    (cd $app && cargo tauri build --target universal-apple-darwin --bundles app,dmg)
    cp target/universal-apple-darwin/release/bundle/dmg/*.dmg "$out/"
    ;;
*) echo "Unknown system $system; say linux, windows or macos" >&2; exit 1 ;;
esac
# File names without spaces ("Xenify Draw_0.1.0_amd64.deb" → xenify-draw_0.1.0_amd64.deb).
for f in "$out"/Xenify\ Draw*; do [ -e "$f" ] && mv "$f" "$out/$(basename "$f" | sed "s/^Xenify Draw/xenify-draw/")"; done
echo "Done:"; ls -lh "$out" | sed -n '2,$p'
