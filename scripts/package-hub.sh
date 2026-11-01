#!/usr/bin/env bash
# Packages Xenify Hub, the one download on the website, into
# dist-hub-installers/ under fixed names the site links to:
#   linux    Xenify-Hub.AppImage, xenify-hub_amd64.deb
#   windows  Xenify-Hub-setup.exe
#   macos    Xenify-Hub.dmg (Apple silicon and Intel)
# Needs the Tauri CLI (cargo install tauri-cli --version "^2" --locked).
set -euo pipefail
cd "$(dirname "$0")/.."
system=${1:?linux, windows or macos}
out=dist-hub-installers
mkdir -p "$out"
app=crates/xenify-hub
case "$system" in
linux)
    (cd $app && cargo tauri build --bundles deb,appimage)
    b=target/release/bundle
    cp "$b"/appimage/Xenify\ Hub*.AppImage "$out/Xenify-Hub.AppImage"
    cp "$b"/deb/Xenify\ Hub*.deb "$out/xenify-hub_amd64.deb"
    ;;
windows)
    target=x86_64-pc-windows-msvc
    [ "$(uname -s)" = Linux ] && target=x86_64-pc-windows-gnu
    (cd $app && cargo tauri build --target $target --bundles nsis)
    cp target/$target/release/bundle/nsis/Xenify\ Hub*.exe "$out/Xenify-Hub-setup.exe"
    ;;
macos)
    rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
    (cd $app && cargo tauri build --target universal-apple-darwin --bundles dmg)
    cp target/universal-apple-darwin/release/bundle/dmg/Xenify\ Hub*.dmg "$out/Xenify-Hub.dmg"
    ;;
esac
ls -la "$out"
