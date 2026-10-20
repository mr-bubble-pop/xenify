#!/usr/bin/env bash
# Puts xenify.net together in target/site, ready for Cloudflare Pages:
#   /         the feature page (site/)
#   /write/   Xenify Write in the browser (target/write-web)
# Builds the web version first unless it's there already (or --rebuild).
# Leaves out the .br/.gz copies: Cloudflare compresses by itself, and the
# copies would double the upload. Deploy with:
#   npx wrangler pages deploy target/site --project-name xenify
set -euo pipefail
cd "$(dirname "$0")/.."
if [ "${1:-}" = "--rebuild" ] || [ ! -f target/write-web/index.html ]; then
    scripts/build-write-web.sh
fi
out=target/site
rm -rf "$out"
mkdir -p "$out/write"
cp -r site/. "$out/"
rsync -a --exclude='*.br' --exclude='*.gz' --exclude='_headers' target/write-web/ "$out/write/"
# The web build's caching rules, moved under /write/, plus the feature page's.
{
    echo "/*"
    echo "  X-Content-Type-Options: nosniff"
    echo "/img/*"
    echo "  Cache-Control: public, max-age=86400"
    echo "/fonts/*"
    echo "  Cache-Control: public, max-age=604800"
    # Cloudflare joins the headers of every matching rule, so the immutable
    # files first drop the no-cache that /write/* gives them ("! Header").
    awk '/^\// { path = $0; print "/write" $0; if (path != "/*") print "  ! Cache-Control"; next } { print }' target/write-web/_headers
} > "$out/_headers"
big=$(find "$out" -type f -size +25M)
if [ -n "$big" ]; then
    echo "Over Cloudflare Pages' 25 MB file limit:" >&2
    echo "$big" >&2
    exit 1
fi
echo "Site ready in $out ($(find "$out" -type f | wc -l) files, $(du -sh "$out" | cut -f1))"
