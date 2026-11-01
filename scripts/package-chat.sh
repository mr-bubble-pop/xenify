#!/usr/bin/env bash
# Packages Xenify Chat (crates/xenify-chat-app) into dist-chat/. Whiteboards
# run Draw's browser version, built in first (scripts/build-chat-draw.sh).
#
#   scripts/package-chat.sh            for this computer's system
#   scripts/package-chat.sh linux      a .tar.gz (unpack and run install.sh),
#                                      a .deb and an Arch PKGBUILD
#   scripts/package-chat.sh windows    the program (the Hub installs it); builds on
#                                      Windows, or on Linux with mingw-w64
#   scripts/package-chat.sh macos      .app and .dmg for Apple silicon and Intel; needs a Mac
#
# Set XENIFY_SERVER=https://chat.example.org to build in the server people
# sign in to (otherwise http://127.0.0.1:7700; invite links set it too).
# scripts/hub-packages.sh chat <system> turns these into the Hub's packages.
set -euo pipefail
cd "$(dirname "$0")/.."
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
app=crates/xenify-chat-app
out=dist-chat
system=${1:-}
if [ -z "$system" ]; then
    case "$(uname -s)" in
        Linux) system=linux ;;
        Darwin) system=macos ;;
        MINGW*|MSYS*|CYGWIN*) system=windows ;;
        *) echo "Unknown system; say linux, windows or macos" >&2; exit 1 ;;
    esac
fi
echo "Packaging Xenify Chat $version for ${system}${XENIFY_SERVER:+ (server $XENIFY_SERVER)}…"
# Every system's icon sizes (in git), made from icons/icon.png when missing.
if [ ! -f $app/icons/icon.ico ] || [ ! -f $app/icons/icon.icns ]; then
    tmp=$(mktemp -d)
    cargo tauri icon $app/icons/icon.png -o "$tmp" >/dev/null
    cp "$tmp"/{32x32.png,128x128.png,128x128@2x.png,icon.ico,icon.icns} $app/icons/
    rm -rf "$tmp"
fi
# Draw for whiteboards, part of the screens the program carries.
scripts/build-chat-draw.sh
# The licences of everything inside (fresh when cargo-about is installed).
if command -v cargo-about >/dev/null; then scripts/licenses.sh; fi
rm -rf "$out" && mkdir -p "$out"

case $system in
linux)
    cargo build --release -p xenify-chat-app -p xenify-checker
    # The tarball (also what the PKGBUILD builds from, and the Hub's package).
    stage="$out/xenify-chat-$version"
    mkdir -p "$stage"
    cp target/release/xenify-chat target/release/xenify-checker "$stage/"
    strip "$stage/xenify-chat" "$stage/xenify-checker" 2>/dev/null || true
    cp $app/packaging/xenify-chat.desktop $app/packaging/install.sh "$stage/"
    cp $app/icons/icon.png "$stage/xenify-chat.png"
    cp LICENSE THIRD_PARTY_LICENSES.txt "$stage/"
    tarball="$out/xenify-chat-$version-$(uname -m)-linux.tar.gz"
    tar -C "$out" -czf "$tarball" "xenify-chat-$version"

    # Arch / Manjaro.
    sum=$(sha256sum "$tarball" | cut -d' ' -f1)
    sed -e "s/@VERSION@/$version/" -e "s/@SHA256@/$sum/" -e "s/@ARCH@/$(uname -m)/" $app/packaging/PKGBUILD.in > "$out/PKGBUILD"

    # Debian / Ubuntu. (Calls need GStreamer's good and bad plugins for the camera.)
    deb="$out/deb"
    mkdir -p "$deb/DEBIAN" "$deb/usr/bin" "$deb/usr/share/applications" "$deb/usr/share/icons/hicolor/256x256/apps" "$deb/usr/share/doc/xenify-chat"
    install -m755 "$stage/xenify-chat" "$stage/xenify-checker" "$deb/usr/bin/"
    install -m644 "$stage/xenify-chat.desktop" "$deb/usr/share/applications/"
    install -m644 "$stage/xenify-chat.png" "$deb/usr/share/icons/hicolor/256x256/apps/"
    install -m644 LICENSE "$stage/THIRD_PARTY_LICENSES.txt" "$deb/usr/share/doc/xenify-chat/"
    size=$(du -sk "$deb/usr" | cut -f1)
    cat > "$deb/DEBIAN/control" <<CONTROL
Package: xenify-chat
Version: $version
Section: net
Priority: optional
Architecture: $(dpkg --print-architecture 2>/dev/null || echo amd64)
Depends: libwebkit2gtk-4.1-0, libgtk-3-0, libsoup-3.0-0, gstreamer1.0-plugins-good, gstreamer1.0-plugins-bad, gstreamer1.0-pipewire
Recommends: xdg-desktop-portal
Installed-Size: $size
Maintainer: Xenify <xenify@localhost>
Description: Chat, email and calendar in one place
 Chat with your team, call them and draw together, read and answer email
 and keep your calendar, with spelling and grammar checked on this computer.
CONTROL
    dpkg-deb --root-owner-group --build "$deb" "$out/xenify-chat_${version}_$(dpkg --print-architecture 2>/dev/null || echo amd64).deb" >/dev/null
    rm -rf "$deb" "$stage"
    ;;
windows)
    # On Linux this cross-builds with mingw-w64 (sudo apt install gcc-mingw-w64-x86-64
    # g++-mingw-w64-x86-64; rustup target add x86_64-pc-windows-gnu).
    target=x86_64-pc-windows-msvc
    voice=()
    if [ "$(uname -s)" = Linux ]; then
        target=x86_64-pc-windows-gnu
        # Without speech to text: whisper.cpp's C++ doesn't link with
        # mingw-w64 (the release build, on Windows itself, has it).
        voice=(--no-default-features)
        echo "Note: built on Linux, this Windows build can't turn speech into text." >&2
    fi
    cargo build --release --target $target -p xenify-chat-app "${voice[@]}"
    cargo build --release --target $target -p xenify-checker
    cp target/$target/release/xenify-chat.exe "$out/"
    ;;
macos)
    command -v cargo-tauri >/dev/null || { echo "Needs Tauri's CLI: cargo install tauri-cli --version '^2' --locked" >&2; exit 1; }
    rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
    # (Bundling is off for everyday builds; the .app is made here.)
    (cd $app && cargo tauri build --target universal-apple-darwin --bundles app,dmg \
        --config '{"bundle":{"active":true,"icon":["icons/32x32.png","icons/128x128.png","icons/128x128@2x.png","icons/icon.icns"]}}')
    cp target/universal-apple-darwin/release/bundle/dmg/*.dmg "$out/"
    for f in "$out"/Xenify\ Chat*; do [ -e "$f" ] && mv "$f" "$out/$(basename "$f" | sed "s/^Xenify Chat/xenify-chat/")"; done
    ;;
*) echo "Unknown system $system; say linux, windows or macos" >&2; exit 1 ;;
esac
echo "Done:"; ls -lh "$out" | sed -n '2,$p'
