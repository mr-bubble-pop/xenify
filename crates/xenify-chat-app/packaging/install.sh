#!/bin/sh
# Installs Xenify Chat from this folder: into ~/.local (just you), or with
# --system into /usr/local (everyone; run with sudo). --remove takes it out.
# Xenify Chat needs the system's web view: webkit2gtk-4.1 (Arch/Manjaro:
# webkit2gtk-4.1; Debian/Ubuntu: libwebkit2gtk-4.1-0; Fedora: webkit2gtk4.1).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
prefix="$HOME/.local"
remove=no
for a in "$@"; do
    case "$a" in
        --system) prefix=/usr/local ;;
        --remove) remove=yes ;;
        *) echo "usage: install.sh [--system] [--remove]"; exit 2 ;;
    esac
done
bin="$prefix/bin"; apps="$prefix/share/applications"; icons="$prefix/share/icons/hicolor/256x256/apps"
if [ "$remove" = yes ]; then
    rm -f "$bin/xenify-chat" "$bin/xenify-checker" "$apps/xenify-chat.desktop" "$icons/xenify-chat.png"
    echo "Xenify Chat removed from $prefix. Your settings, messages and email copies stay in ~/.config/xenify and ~/.local/share/xenify."
    exit 0
fi
mkdir -p "$bin" "$apps" "$icons"
install -m 755 "$here/xenify-chat" "$here/xenify-checker" "$bin/"
install -m 644 "$here/xenify-chat.png" "$icons/xenify-chat.png"
# The menu entry runs Xenify Chat from where it was installed.
sed "s|^Exec=xenify|Exec=$bin/xenify|" "$here/xenify-chat.desktop" > "$apps/xenify-chat.desktop"
chmod 644 "$apps/xenify-chat.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database -q "$apps" 2>/dev/null || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$prefix/share/icons/hicolor" 2>/dev/null || true
# Invite links (xenify://join?...) open Xenify Chat.
command -v xdg-mime >/dev/null && xdg-mime default xenify-chat.desktop x-scheme-handler/xenify 2>/dev/null || true
if ! ldconfig -p 2>/dev/null | grep -q libwebkit2gtk-4.1; then
    echo "Note: webkit2gtk-4.1 doesn't seem to be installed; Xenify Chat needs it (see the top of this file)."
fi
echo "Xenify Chat is installed in $bin. Find it in your applications menu, or run: $bin/xenify-chat"
