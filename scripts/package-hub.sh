#!/usr/bin/env bash
# Packages Xenify Hub, the one download on the website, into
# dist-hub-installers/ under fixed names the site links to:
#   linux    Xenify-Hub.AppImage, xenify-hub_amd64.deb
#   windows  Xenify-Hub-setup.exe
#   macos    Xenify-Hub.dmg (Apple silicon and Intel)
# and, for the Hub to update itself, into dist-hub/ as the release list's
# product "hub" (scripts/release.sh adds them):
#   hub-<version>-windows-x86_64.exe     the same installer (run quietly)
#   hub-<version>-linux-x86_64.AppImage  the same AppImage (swapped in)
#   hub-<version>-macos-<arch>.tar.gz    the app (one copy for both kinds of Mac)
#
#   scripts/package-hub.sh <linux|windows|macos> [version]
#
# The version (default: the workspace's) is built into the Hub, so it knows
# where it stands in the release list: a nightly is 0.1.5-nightly.<time>.
# Needs the Tauri CLI (cargo install tauri-cli --version "^2" --locked).
set -euo pipefail
cd "$(dirname "$0")/.."
system=${1:?linux, windows or macos}
version=${2:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}
export XENIFY_VERSION=$version
out=dist-hub-installers
mkdir -p "$out" dist-hub
app=crates/xenify-hub
# The file just built (an earlier version's may still be in the folder).
newest() { ls -t "$@" | head -n 1; }
case "$system" in
linux)
    (cd $app && cargo tauri build --bundles deb,appimage)
    b=target/release/bundle
    cp "$(newest "$b"/appimage/Xenify\ Hub*.AppImage)" "$out/Xenify-Hub.AppImage"
    cp "$(newest "$b"/deb/Xenify\ Hub*.deb)" "$out/xenify-hub_amd64.deb"
    cp "$out/Xenify-Hub.AppImage" "dist-hub/hub-$version-linux-x86_64.AppImage"
    ;;
windows)
    target=x86_64-pc-windows-msvc
    [ "$(uname -s)" = Linux ] && target=x86_64-pc-windows-gnu
    (cd $app && cargo tauri build --target $target --bundles nsis)
    cp "$(newest target/$target/release/bundle/nsis/Xenify\ Hub*.exe)" "$out/Xenify-Hub-setup.exe"
    cp "$out/Xenify-Hub-setup.exe" "dist-hub/hub-$version-windows-x86_64.exe"
    ;;
macos)
    rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
    (cd $app && cargo tauri build --target universal-apple-darwin --bundles app,dmg)
    b=target/universal-apple-darwin/release/bundle
    cp "$(newest "$b"/dmg/Xenify\ Hub*.dmg)" "$out/Xenify-Hub.dmg"
    tar -C "$b/macos" -czf "dist-hub/hub-$version-macos-aarch64.tar.gz" "Xenify Hub.app"
    cp "dist-hub/hub-$version-macos-aarch64.tar.gz" "dist-hub/hub-$version-macos-x86_64.tar.gz"
    ;;
esac
ls -la "$out" dist-hub/hub-* 2>/dev/null
