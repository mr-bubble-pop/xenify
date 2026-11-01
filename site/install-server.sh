#!/bin/sh
# Installs the Xenify server on a Linux computer without a screen (a VPS,
# or a home server), as a service that starts with the computer and keeps
# itself up to date. Run it as root:
#
#   curl -fsSL https://xenify.net/install-server.sh | sudo sh
#
# With a domain name pointing at this computer, the server gets a
# certificate from Let's Encrypt and its pages open over HTTPS:
#
#   curl -fsSL https://xenify.net/install-server.sh | sudo sh -s -- --domain files.example.com --email you@example.com
#
# Without one, nothing is opened to the internet for setting up: the script
# prints an SSH tunnel command (or use a VPN such as Tailscale or WireGuard).
# Setting up needs the one-time link this script prints.
#
# The database: by default a private PostgreSQL just for Xenify (a portable
# build from Xenify's signed release list, kept in /var/lib/xenify-server,
# reachable from this computer only, started by the service). Nothing is
# installed on the system. To use a PostgreSQL you already run instead:
#   --database postgres://user:password@host/dbname
# (Run from a terminal, it asks which.)
#
# Encryption at rest: --encrypt keeps the database and people's files in a
# folder the computer encrypts (ext4; the disk's encryption feature is
# turned on). After each restart the server stays locked until you run
#   sudo -u xenify /opt/xenify-server/xenify-server --config /etc/xenify-server/config.toml unlock
# The passphrase is printed once, at the end: write it down.
#
# Other options: --from <xenify-server package .tar.gz> (instead of
# downloading it; you vouch for that file), --skip-dns-check (don't check
# that --domain points here). Running it again keeps the database and settings.
#
# The download is checked against Xenify's signed release list (the same
# list Xenify Hub and the server's updates use), with the release key's
# public half written below: nothing is installed if that check fails.
#
# Linux with systemd, x86_64; needs curl, tar and OpenSSL 3 or newer.
set -eu

DOMAIN=""
EMAIL=""
FROM=""
DATABASE=""
DNS_CHECK=1
ENCRYPT=""
PASSPHRASE=""
RELEASES="https://github.com/mr-bubble-pop/xenify-releases/releases/download/hub-releases/manifest.json"
# Xenify's release key, the public half (as xenify_hub_core::TRUSTED), and
# its id (the key's first 8 bytes, hex) as the signed list names it.
RELEASE_KEY="MCowBQYDK2VwAyEAJHV/+KXNYIB8mAcDDo90b7Ro+HQ6+9VE/qcDtehoL6U="
RELEASE_KEY_ID="24757ff8a5cd6080"
DIR=/opt/xenify-server
DATA=/var/lib/xenify-server
CONFIG=/etc/xenify-server/config.toml

say() { printf '%s\n' "$*"; }
fail() { printf 'xenify-server install: %s\n' "$*" >&2; exit 1; }

while [ $# -gt 0 ]; do
    case "$1" in
        --domain) DOMAIN=${2:?--domain needs a name}; shift 2 ;;
        --email) EMAIL=${2:?--email needs an address}; shift 2 ;;
        --from) FROM=${2:?--from needs a file}; shift 2 ;;
        --database) DATABASE=${2:?--database needs an address}; shift 2 ;;
        --skip-dns-check) DNS_CHECK=""; shift ;;
        --encrypt) ENCRYPT=1; shift ;;
        *) fail "unknown option $1 (see the top of this script)" ;;
    esac
done

[ "$(id -u)" = 0 ] || fail "run it as root (with sudo)"
[ "$(uname -s)" = Linux ] || fail "this is for Linux"
[ "$(uname -m)" = x86_64 ] || fail "there's no server build for $(uname -m) yet"
command -v systemctl >/dev/null || fail "this needs systemd"

for tool in curl tar sha256sum runuser awk sed sort base64; do
    command -v "$tool" >/dev/null || fail "this needs $tool"
done

# ---- The domain points here? -----------------------------------------------
# Let's Encrypt checks the domain by connecting to it on port 443, so it must
# lead to this computer itself, not to a proxy in front of it.
if [ -n "$DOMAIN" ] && [ -n "$DNS_CHECK" ]; then
    names=$(getent ahosts "$DOMAIN" 2>/dev/null | awk '{print $1}' | sort -u)
    [ -n "$names" ] || fail "$DOMAIN doesn't lead anywhere yet: add a DNS record for it pointing at this computer, or wait a few minutes after adding one"
    # This computer's own public addresses, as the internet sees them.
    here4=$(curl -fsS4 --max-time 10 https://1.1.1.1/cdn-cgi/trace 2>/dev/null | sed -n 's/^ip=//p')
    here6=$(curl -fsS6 --max-time 10 'https://[2606:4700:4700::1111]/cdn-cgi/trace' 2>/dev/null | sed -n 's/^ip=//p')
    matched=""
    proxied=""
    for a in $names; do
        if [ "$a" = "$here4" ] || [ "$a" = "$here6" ]; then matched=1; fi
        case "$a" in
            104.1[6-9].*|104.2[0-3].*|172.6[4-9].*|172.7[01].*|188.114.9[6-9].*|162.15[89].*|2606:4700:*|2803:f800:*|2a06:98c0:*) proxied=1 ;;
        esac
    done
    if [ -z "$matched" ]; then
        say "$DOMAIN leads to $(echo $names | tr ' ' ',') but this computer is ${here4:-?}${here6:+ / $here6}."
        if [ -n "$proxied" ]; then
            say "Those are Cloudflare's addresses: its proxy is on for this name. In Cloudflare's DNS page,"
            say "set the record for $DOMAIN to \"DNS only\" (the grey cloud), wait a minute, then run this again."
        else
            say "Point $DOMAIN's DNS record at ${here4:-this computer's address} (and wait a few minutes), then run this again."
        fi
        fail "the certificate can't be made until $DOMAIN points here (--skip-dns-check if you're sure it does)"
    fi
fi

# ---- Its own user and folders ------------------------------------------
if ! id xenify >/dev/null 2>&1; then
    useradd --system --home-dir "$DATA" --shell "$(command -v nologin || echo /bin/false)" xenify
fi
install -d -m 700 -o xenify -g xenify "$DATA" "$DATA/data"
install -d -m 755 "$DIR"
install -d -m 750 -o root -g xenify "$(dirname "$CONFIG")"

# ---- The server ----------------------------------------------------------
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
chmod 755 "$work"

# The signed release list: its signature (ed25519, with the key above),
# then the server's newest release in it, and that download's checksum.
# Prints "<version> <url> <sha256> <program>"; fails when anything is off.
signed_server() {
    case "$(openssl version 2>/dev/null)" in
        "OpenSSL "[3-9].*|"OpenSSL "[1-9][0-9].*) ;;
        *) fail "this needs OpenSSL 3 or newer (to check the release list's signature): install openssl and run this again" ;;
    esac
    curl -fsSL -o "$work/signed.json" "$RELEASES" || fail "couldn't download the release list ($RELEASES)"
    key=$(sed -n 's/^ *"key": *"\([0-9a-f]*\)",*$/\1/p' "$work/signed.json")
    sig=$(sed -n 's/^ *"signature": *"\([A-Za-z0-9+\/=]*\)",*$/\1/p' "$work/signed.json")
    [ "$key" = "$RELEASE_KEY_ID" ] || fail "the release list is signed with a key this script doesn't know"
    # The list's exact text, from inside its JSON string.
    LC_ALL=C awk '
        function hexval(h,   i, v) { v = 0; for (i = 1; i <= length(h); i++) v = v * 16 + index("0123456789abcdef", tolower(substr(h, i, 1))) - 1; return v }
        function utf8(c) {
            if (c < 128) printf "%c", c
            else if (c < 2048) printf "%c%c", 192 + int(c / 64), 128 + c % 64
            else printf "%c%c%c", 224 + int(c / 4096), 128 + int(c / 64) % 64, 128 + c % 64
        }
        /^ *"manifest": *"/ {
            s = $0
            sub(/^ *"manifest": *"/, "", s)
            sub(/",?[ \t\r]*$/, "", s)
            while ((i = index(s, "\\")) > 0) {
                printf "%s", substr(s, 1, i - 1)
                c = substr(s, i + 1, 1)
                if (c == "n") printf "\n"
                else if (c == "t") printf "\t"
                else if (c == "r") printf "\r"
                else if (c == "b") printf "\b"
                else if (c == "f") printf "\f"
                else if (c == "u") { utf8(hexval(substr(s, i + 2, 4))); s = substr(s, i + 6); continue }
                else printf "%s", c
                s = substr(s, i + 2)
            }
            printf "%s", s
            found = 1
        }
        END { exit !found }' "$work/signed.json" > "$work/list.json" || fail "the release list can't be read"
    printf '%s\n' "-----BEGIN PUBLIC KEY-----" "$RELEASE_KEY" "-----END PUBLIC KEY-----" > "$work/release-key.pem"
    printf '%s' "$sig" | base64 -d > "$work/list.sig" 2>/dev/null || fail "the release list's signature can't be read"
    openssl pkeyutl -verify -pubin -inkey "$work/release-key.pem" -rawin -in "$work/list.json" -sigfile "$work/list.sig" >/dev/null 2>&1 ||
        fail "the release list's signature doesn't match: it was changed after signing. Nothing was installed."
    # Not older than one this computer saw before.
    published=$(sed -n 's/^  "published": "\([^"]*\)",*$/\1/p' "$work/list.json")
    seen=$(cat "$DATA/release-list-seen" 2>/dev/null || true)
    if [ -n "$seen" ] && awk -v a="$published" -v b="$seen" 'BEGIN { exit !(a < b) }'; then
        fail "the release list is older than one this computer had before ($seen)"
    fi
    printf '%s\n' "$published" > "$DATA/release-list-seen"
    # The server's releases for this computer, and its oldest safe version.
    LC_ALL=C awk '
        function val(line) { sub(/^ *"[a-z_0-9]*": */, "", line); sub(/,$/, "", line); gsub(/^"|"$/, "", line); return line }
        /^      "id": /           { pid = val($0) }
        /^      "min_safe": /     { if (pid == "server") floor = val($0) }
        /^          "version": /  { ver = val($0) }
        /^          "channel": /  { ch = val($0) }
        /^              "os": /   { os = val($0) }
        /^              "arch": / { arch = val($0) }
        /^              "url": /  { url = val($0) }
        /^              "sha256": / { sha = val($0) }
        /^              "run": /  { run = val($0) }
        /^            }/ {
            if (pid == "server" && ch == "release" && os == "linux" && arch == "x86_64") print ver, url, sha, run
            os = arch = url = sha = run = ""
        }
        END { print "floor", floor }' "$work/list.json" > "$work/server.txt"
    floor=$(sed -n 's/^floor *//p' "$work/server.txt")
    best=$(grep -v '^floor' "$work/server.txt" | sort -V -k1,1 | tail -n 1)
    [ -n "$best" ] || fail "the release list has no server for this computer"
    version=${best%% *}
    if [ -n "$floor" ] && [ "$(printf '%s\n%s\n' "$floor" "$version" | sort -V | head -n 1)" != "$floor" ]; then
        fail "the newest server in the release list ($version) is older than its oldest safe version ($floor)"
    fi
    printf '%s\n' "$best"
}

run="xenify-server/xenify-server"
if [ -n "$FROM" ]; then
    cp "$FROM" "$work/server.tar.gz"
else
    say "Checking Xenify's signed release list…"
    found=$(signed_server) || exit 1
    set -- $found
    [ $# -eq 4 ] || fail "the release list can't be read"
    url=$2 sha=$3 run=$4
    case "$sha" in *[!0-9a-f]*|"") fail "the release list's checksum for the server isn't one" ;; esac
    case "$url" in https://*) ;; *) fail "the release list's address for the server isn't https" ;; esac
    say "Downloading the server ($1)…"
    curl -fsSL -o "$work/server.tar.gz" "$url" || fail "couldn't download $url"
    echo "$sha  $work/server.tar.gz" | sha256sum -c - >/dev/null 2>&1 ||
        fail "the download doesn't match the signed release list. Nothing was installed."
fi
case "$run" in /*|*..*|"") fail "the release list names an odd program" ;; esac
tar -xzf "$work/server.tar.gz" -C "$work"
program="$work/$run"
[ -x "$program" ] || fail "the package has no xenify-server program"

# ---- Its database and settings (made once) -------------------------------
if [ ! -f "$CONFIG" ]; then
    # A terminal to ask on (not when run without one, e.g. over plain ssh).
    if [ -z "$DATABASE" ] && ( : < /dev/tty ) 2>/dev/null; then
        {
            say ""
            say "The server keeps its data in PostgreSQL."
            say "  1) Set up a private PostgreSQL just for Xenify (recommended)"
            say "  2) Use a PostgreSQL you already run"
            printf "Which? [1] "
        } > /dev/tty
        read -r answer < /dev/tty || answer=1
        if [ "$answer" = 2 ]; then
            printf "Its address (postgres://user:password@host/dbname): " > /dev/tty
            read -r DATABASE < /dev/tty
            [ -n "$DATABASE" ] || fail "no address given"
        fi
    fi
    pg_dir="$DATA/postgres"
    at_rest_lines=""
    if [ -n "$ENCRYPT" ]; then
        [ -z "$DATABASE" ] || fail "--encrypt needs the server's own PostgreSQL (its data goes in the encrypted folder)"
        dev=$(df --output=source "$DATA" | tail -1)
        [ "$(df --output=fstype "$DATA" | tail -1)" = ext4 ] || fail "--encrypt needs $DATA on an ext4 disk"
        if ! tune2fs -l "$dev" 2>/dev/null | grep -q '^Filesystem features:.* encrypt'; then
            say "Turning on encryption for $dev…"
            tune2fs -O encrypt "$dev" >/dev/null || fail "couldn't turn on encryption for $dev"
        fi
        # Linux 7.0 (Ubuntu 26.04) can crash writing encrypted files with
        # ext4's dioread_nolock (the default): turn it off for this disk, now
        # and in /etc/fstab.
        mnt=$(findmnt -n -o TARGET --target "$DATA")
        mount -o remount,nodioread_nolock "$mnt" || fail "couldn't remount $mnt with nodioread_nolock"
        if ! awk -v m="$mnt" '$2 == m && $4 ~ /nodioread_nolock/ { found = 1 } END { exit !found }' /etc/fstab; then
            cp /etc/fstab /etc/fstab.before-xenify
            awk -v m="$mnt" 'BEGIN { OFS = "\t" } $1 !~ /^#/ && $2 == m { $4 = $4 ",nodioread_nolock" } { print }' /etc/fstab.before-xenify > /etc/fstab
            say "Added nodioread_nolock to $mnt in /etc/fstab (the old one is /etc/fstab.before-xenify)."
        fi
        say "Making the encrypted folder…"
        made=$(cd / && runuser -u xenify -- "$program" at-rest setup --dir "$DATA/secure" --generate) || fail "couldn't make the encrypted folder"
        PASSPHRASE=$(printf '%s\n' "$made" | sed -n 's/^    \([a-z0-9-]*\)$/\1/p' | head -1)
        pg_dir="$DATA/secure/postgres"
        at_rest_lines=$(printf '[at_rest]\ndir = "%s"' "$DATA/secure")
    fi
    if [ -n "$DATABASE" ]; then
        db_lines="database = \"$DATABASE\""
        pg_lines=""
    else
        say "Setting up its own PostgreSQL…"
        install -d -m 700 -o xenify -g xenify "$pg_dir"
        made=$(cd / && runuser -u xenify -- "$program" database setup --dir "$pg_dir") || fail "couldn't set up PostgreSQL"
        db_lines=$(printf '%s\n' "$made" | sed -n '1p')
        pg_lines=$(printf '%s\n' "$made" | sed -n '2,$p')
    fi
    umask 027
    {
        echo "# Made by install-server.sh. Everything else is on the admin pages."
        echo "$db_lines"
        echo "data = \"$DATA/data\""
        echo "# Plain HTTP from this computer only (an SSH tunnel); everyone else uses HTTPS."
        echo "listen = \"127.0.0.1:7700\""
        [ -z "$pg_lines" ] || { echo "# Its own PostgreSQL, started by the service."; echo "$pg_lines"; }
        [ -z "$at_rest_lines" ] || { echo "# Encryption at rest: locked after each start until: xenify-server unlock"; echo "$at_rest_lines"; }
    } > "$CONFIG"
    umask 022
    chown root:xenify "$CONFIG"
    chmod 640 "$CONFIG"
fi

# Settings before the first start (later: Admin › Network).
if [ -n "$DOMAIN" ]; then
    runuser -u xenify -- "$program" --config "$CONFIG" configure --domain "$DOMAIN" --email "$EMAIL" --https-port 443 --admin-access anywhere
    if command -v ufw >/dev/null && ufw status 2>/dev/null | grep -q 'Status: active'; then
        ufw allow 443/tcp >/dev/null
    elif command -v firewall-cmd >/dev/null && firewall-cmd --state >/dev/null 2>&1; then
        firewall-cmd --permanent --add-service=https >/dev/null && firewall-cmd --reload >/dev/null
    fi
else
    runuser -u xenify -- "$program" --config "$CONFIG" configure >/dev/null
fi

say "Starting it with the computer…"
was=$(systemctl is-active xenify-server 2>/dev/null || true)
"$program" --config "$CONFIG" service install --dir "$DIR" --user xenify
[ "$was" = active ] && systemctl restart xenify-server

# ---- The setup link --------------------------------------------------------
for _ in $(seq 1 60); do
    [ -f "$DATA/data/address.json" ] && break
    sleep 1
done
setup=$(sed -n 's/.*"setup":"\([^"]*\)".*/\1/p' "$DATA/data/address.json" 2>/dev/null || true)
again="sudo -u xenify $DIR/xenify-server --config $CONFIG setup-link"
say ""
if [ -z "$setup" ]; then
    if [ -f "$DATA/data/address.json" ]; then
        say "The Xenify server is running and was set up before."
    else
        say "The server hasn't started yet: see  journalctl -u xenify-server"
    fi
elif [ -n "$DOMAIN" ]; then
    say "The Xenify server is running. To set it up, open:"
    say ""
    say "    https://$DOMAIN$setup"
    say ""
    say "$DOMAIN must point at this computer (port 443 open). The certificate"
    say "is made on the first visit, which can take a minute."
else
    ip=$(hostname -I 2>/dev/null | awk '{print $1}')
    say "The Xenify server is running. Its admin pages aren't open to the internet."
    say "To set it up, run this on your own computer:"
    say ""
    say "    ssh -L 7700:localhost:7700 ${SUDO_USER:-root}@${ip:-this-server}"
    say ""
    say "and while that's connected, open:"
    say ""
    say "    http://localhost:7700$setup"
    say ""
    say "(Or, over a VPN such as Tailscale or WireGuard: https://<its VPN address>:7443$setup)"
fi
say ""
say "The link works once. To see it again: $again"
if [ -n "$PASSPHRASE" ]; then
    say ""
    say "Encryption at rest is on. The key to the encrypted folder:"
    say ""
    say "    $PASSPHRASE"
    say ""
    say "Write it down now: it isn't kept anywhere, and without it nobody can read"
    say "the files, not even us. After each restart, unlock the server with:"
    say ""
    say "    sudo -u xenify $DIR/xenify-server --config $CONFIG unlock"
fi
