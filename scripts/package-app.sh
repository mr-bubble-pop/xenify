#!/usr/bin/env bash
# Builds Xenify Chat (chat, email and calendar) and packages it into dist/:
#   xenify-chat-<version>-x86_64.tar.gz  any Linux: unpack and run install.sh
#   xenify-chat_<version>_amd64.deb Debian, Ubuntu: sudo apt install ./xenify-chat_…deb
#   PKGBUILD                        Arch, Manjaro: makepkg -si next to the tarball
# Set XENIFY_SERVER=https://chat.example.org to build in the server people
# sign in to (otherwise http://127.0.0.1:7700; invite links set it too).
set -euo pipefail
cd "$(dirname "$0")/.."
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
app=crates/xenify-chat-app
out=dist
echo "Building Xenify Chat $version${XENIFY_SERVER:+ for $XENIFY_SERVER}…"
cargo build --release -p xenify-chat-app -p xenify-checker
rm -rf "$out" && mkdir -p "$out"

# The tarball (also what the PKGBUILD builds from).
stage="$out/xenify-chat-$version"
mkdir -p "$stage"
cp target/release/xenify-chat target/release/xenify-checker "$stage/"
strip "$stage/xenify-chat" "$stage/xenify-checker" 2>/dev/null || true
cp $app/packaging/xenify-chat.desktop $app/packaging/install.sh "$stage/"
cp $app/icons/icon.png "$stage/xenify-chat.png"
cp LICENSE "$stage/"
# The licences of everything inside (fresh when cargo-about is installed).
if command -v cargo-about >/dev/null; then scripts/licenses.sh; fi
cp THIRD_PARTY_LICENSES.txt "$stage/"
tar -C "$out" -czf "$out/xenify-chat-$version-x86_64.tar.gz" "xenify-chat-$version"

# Arch / Manjaro.
sum=$(sha256sum "$out/xenify-chat-$version-x86_64.tar.gz" | cut -d' ' -f1)
sed -e "s/@VERSION@/$version/" -e "s/@SHA256@/$sum/" $app/packaging/PKGBUILD.in > "$out/PKGBUILD"

# Debian / Ubuntu.
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
Architecture: amd64
Depends: libwebkit2gtk-4.1-0, libgtk-3-0, libsoup-3.0-0
Installed-Size: $size
Maintainer: Xenify <xenify@localhost>
Description: Chat, email and calendar in one place
 Chat with your team, read and answer email and keep your calendar,
 with spelling and grammar checked on this computer.
CONTROL
dpkg-deb --root-owner-group --build "$deb" "$out/xenify-chat_${version}_amd64.deb" >/dev/null
rm -rf "$deb"
echo "Done:"; ls -lh "$out" | sed -n '2,$p'
