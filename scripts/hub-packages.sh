#!/usr/bin/env bash
# Makes Xenify Hub's packages from a finished build, named as
# scripts/release.sh wants them (<app>-<version>-<os>-<arch>.<ext>), each
# with a <package>.run file naming the program inside it.
#
#   scripts/hub-packages.sh <write|draw|calc|chat|plan|server> <linux|windows|macos> [version]
#   scripts/hub-packages.sh checker [version]
#   scripts/hub-packages.sh web [version]
#
# Write, Draw, Calc and Chat: run after scripts/package-<app>.sh <system>, which
# builds them (Calc has no spelling checker, so no model to leave out). Their packages leave the mistake-spotter model out: the Hub installs
# it once for both as the "checker" part (`checker` packages it, the same
# file for every system: dist-hub/checker-<version>.tar.gz, default version
# $CHECKER_VERSION or 7.0.0 for model B7). The server is built here. The version defaults to Cargo.toml's (a
# Preview or Nightly build passes its own, e.g. 1.2.0-nightly.20261024).
# `web`: the apps for the browser (Write, Draw, Calc and Plan as
# scripts/build-site.sh builds them in target/site, and Chat from
# target/site-parts/chat: it isn't on xenify.net), for a Xenify server to
# download and offer at its address (Admin › Network): one package for
# every computer, dist-hub/web-<version>-any-any.tar.gz. Run build-site.sh
# first.
# Packages go to dist-hub/.
set -euo pipefail
cd "$(dirname "$0")/.."

app=${1:?write, draw, calc, chat, plan, server, checker or web}
if [ "$app" = checker ]; then
    version=${2:-${CHECKER_VERSION:-7.0.0}}
    scripts/stage-spotter.sh
    stage=$(mktemp -d)
    trap 'rm -rf "$stage"' EXIT
    mkdir -p "$stage/spotter" dist-hub
    cp target/spotter/config.json target/spotter/tokenizer.json target/spotter/model.f16.safetensors target/spotter/README.md target/spotter/LICENSE-DeBERTa.txt "$stage/spotter/"
    tar -C "$stage" -czf "dist-hub/checker-$version.tar.gz" spotter
    echo "made dist-hub/checker-$version.tar.gz (runs spotter/config.json)"
    exit 0
fi
if [ "$app" = web ]; then
    version=${2:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}
    from() { [ "$1" = chat ] && echo target/site-parts/chat || echo "target/site/$1"; }
    for a in write draw calc plan chat shared; do
        [ -d "$(from $a)" ] || { echo "$(from $a) is missing: run scripts/build-site.sh first" >&2; exit 1; }
    done
    stage=$(mktemp -d)
    trap 'rm -rf "$stage"' EXIT
    mkdir -p dist-hub
    # Cloudflare's caching rules and precompressed copies aren't used here.
    for a in write draw calc plan chat shared; do
        rsync -a --exclude='_headers' --exclude='*.br' --exclude='*.gz' "$(from $a)/" "$stage/$a/"
    done
    cp crates/xenify-server/web-apps/index.html "$stage/"
    name="web-$version-any-any.tar.gz"
    tar -C "$stage" -czf "dist-hub/$name" .
    printf '%s\n' index.html > "dist-hub/$name.run"
    echo "made dist-hub/$name (runs index.html)"
    exit 0
fi
system=${2:?linux, windows or macos}
cargo_version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
version=${3:-$cargo_version}
out=dist-hub
mkdir -p "$out"
arch=x86_64
[ "$(uname -m)" = aarch64 ] || [ "$(uname -m)" = arm64 ] && arch=aarch64
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

# The package, and the program inside it to start.
put() { # file run [arch]
    local name="$app-$version-$system-${3:-$arch}.${1##*.}"
    case "$1" in *.tar.gz) name="$app-$version-$system-${3:-$arch}.tar.gz" ;; esac
    cp "$1" "$out/$name"
    printf '%s\n' "$2" > "$out/$name.run"
    echo "made $out/$name (runs $2)"
}

if [ "$app" = server ]; then
    exe=xenify-server; [ "$system" = windows ] && exe=xenify-server.exe
    # Its version is built in (updates and the browser apps go by it).
    XENIFY_VERSION=$version cargo build --release -p xenify-server
    # Servers have no desktop libraries: a build that needs the system's fonts
    # (fontconfig, freetype) wouldn't start on one (it happened: 2026-11-01).
    if [ "$system" = linux ] && ldd "target/release/$exe" | grep -Eq 'fontconfig|freetype|libX|wayland|gtk'; then
        echo "xenify-server needs desktop libraries:" >&2; ldd "target/release/$exe" | grep -E 'fontconfig|freetype|libX|wayland|gtk' >&2; exit 1
    fi
    mkdir -p "$stage/xenify-server"
    cp "target/release/$exe" "$stage/xenify-server/"
    # The admin pages, served from beside the program.
    cp -r crates/xenify-server/web "$stage/xenify-server/web"
    cp LICENSE THIRD_PARTY_LICENSES.txt "$stage/xenify-server/"
    if [ "$system" = windows ]; then
        (cd "$stage" && 7z a -tzip -bd -y server.zip xenify-server >/dev/null)
        put "$stage/server.zip" "xenify-server/$exe"
    else
        tar -C "$stage" -czf "$stage/server.tar.gz" xenify-server
        put "$stage/server.tar.gz" "xenify-server/$exe"
    fi
    exit 0
fi

case "$app" in write) name="Xenify Write" ;; draw) name="Xenify Draw" ;; calc) name="Xenify Calc" ;; chat) name="Xenify Chat" ;; plan) name="Xenify Plan" ;; *) name="Xenify $app" ;; esac
case "$system" in
linux)
    # package-<app>.sh made a portable tarball: xenify-<app>-<version>/xenify-<app>
    # (repacked without the model, which the Hub shares).
    tar -C "$stage" -xzf "dist-$app/xenify-$app-$cargo_version-$(uname -m)-linux.tar.gz"
    rm -rf "$stage/xenify-$app-$cargo_version/spotter"
    tar -C "$stage" -czf "$stage/$app.tar.gz" "xenify-$app-$cargo_version"
    put "$stage/$app.tar.gz" "xenify-$app-$cargo_version/xenify-$app"
    ;;
windows)
    # The program with what sits beside it (no installer: the Hub unpacks it).
    target=x86_64-pc-windows-msvc
    [ "$(uname -s)" = Linux ] && target=x86_64-pc-windows-gnu
    rel="target/$target/release"
    mkdir -p "$stage/xenify-$app"
    cp "$rel/xenify-$app.exe" "$stage/xenify-$app/"
    cp "$rel/xenify-checker.exe" "$stage/xenify-$app/" 2>/dev/null || true
    cp LICENSE THIRD_PARTY_LICENSES.txt "$stage/xenify-$app/"
    (cd "$stage" && 7z a -tzip -bd -y "$app.zip" "xenify-$app" >/dev/null)
    put "$stage/$app.zip" "xenify-$app/xenify-$app.exe"
    ;;
macos)
    # The universal app bundle (Apple silicon and Intel alike).
    tar -C target/universal-apple-darwin/release/bundle/macos --exclude "$name.app/Contents/Resources/spotter" -czf "$stage/$app.tar.gz" "$name.app"
    put "$stage/$app.tar.gz" "$name.app/Contents/MacOS/xenify-$app" aarch64
    put "$stage/$app.tar.gz" "$name.app/Contents/MacOS/xenify-$app" x86_64
    ;;
esac
