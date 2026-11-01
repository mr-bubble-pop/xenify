#!/usr/bin/env bash
# Puts xenify.net together in target/site, ready for Cloudflare Pages:
#   /                 the home page (site/), with /products/write/,
#                     /products/draw/, /products/calc/, /products/plan/,
#                     /products/chat/, /products/server/
#                     and /download/
#                     (only Xenify Hub is offered there)
#   /write/           Xenify Write in the browser
#   /draw/            Xenify Draw in the browser
#   /calc/            Xenify Calc in the browser (the shared checker for its text cells)
#   /plan/            Xenify Plan in the browser (the shared checker for summaries, descriptions and comments)
#   /shared/checker/  the spelling and grammar checker and its model, used by
#                     Write, Draw and Calc and kept in the browser once for all
# The apps are built for the site into target/site-parts (the checker left
# out of each). Chat is built there too but isn't on xenify.net: it needs a
# server, so only servers offer it (hub-packages.sh web takes it from
# target/site-parts/chat); --rebuild builds them again even when they're there already.
# Leaves out the .br/.gz copies: Cloudflare compresses by itself, and the
# copies would double the upload. Deploy with:
#   npx wrangler pages deploy target/site --project-name xenify
set -euo pipefail
cd "$(dirname "$0")/.."
parts=target/site-parts
if [ "${1:-}" = "--rebuild" ] || [ ! -f $parts/write/index.html ] || [ ! -f $parts/draw/index.html ] || [ ! -f $parts/calc/index.html ] || [ ! -f $parts/plan/index.html ] || [ ! -f target/checker-web/kept.js ]; then
    scripts/build-checker-web.sh
    OUT=$parts/write CHECKER_AT=../shared/checker/ SKIP_CHECKER=1 scripts/build-write-web.sh
    OUT=$parts/draw CHECKER_AT=../shared/checker/ SKIP_CHECKER=1 scripts/build-draw-web.sh
    OUT=$parts/calc CHECKER_AT=../shared/checker/ SKIP_CHECKER=1 scripts/build-calc-web.sh
    OUT=$parts/plan CHECKER_AT=../shared/checker/ SKIP_CHECKER=1 scripts/build-plan-web.sh
fi
# Chat is small and follows the core closely: built each time.
OUT=$parts/chat DRAW_AT=../draw/ CHECKER_AT=../shared/checker/ scripts/build-chat-web.sh
out=target/site
rm -rf "$out"
mkdir -p "$out/write" "$out/draw" "$out/calc" "$out/plan" "$out/shared/checker"
cp -r site/. "$out/"
for app in write draw calc plan; do
    rsync -a --exclude='*.br' --exclude='*.gz' --exclude='_headers' $parts/$app/ "$out/$app/"
done
rsync -a --exclude='files.json' target/checker-web/ "$out/shared/checker/"
# Each app's caching rules, moved under its folder, plus the feature page's
# and the checker's.
{
    echo "/*"
    echo "  X-Content-Type-Options: nosniff"
    echo "/img/*"
    echo "  Cache-Control: public, max-age=86400"
    echo "/fonts/*"
    echo "  Cache-Control: public, max-age=604800"
    # The pages' own style and script (and the server installer) change
    # with every deploy, so they're checked each time (a cheap ETag check).
    for file in /style.css /site.js /install-server.sh; do
        echo "$file"
        echo "  Cache-Control: no-cache"
    done
    # Cloudflare joins the headers of every matching rule, so the immutable
    # files first drop the no-cache that /<app>/* gives them ("! Header").
    for app in write draw calc plan; do
        awk -v app="/$app" '/^\// { path = $0; print app $0; if (path != "/*") print "  ! Cache-Control"; next } { print }' $parts/$app/_headers
    done
    # kept.js lists the checker's files, so it's checked each time; the rest
    # is fetched by content hash.
    echo "/shared/*"
    echo "  Cache-Control: no-cache"
    echo "/shared/checker/*.wasm"
    echo "  ! Cache-Control"
    echo "  Cache-Control: public, max-age=31536000, immutable"
    echo "/shared/checker/spotter/model.part*"
    echo "  ! Cache-Control"
    echo "  Cache-Control: public, max-age=31536000, immutable"
} > "$out/_headers"
big=$(find "$out" -type f -size +25M)
if [ -n "$big" ]; then
    echo "Over Cloudflare Pages' 25 MB file limit:" >&2
    echo "$big" >&2
    exit 1
fi
# Opened as people will meet it (served as Cloudflare serves it; each app
# again and again, and offline) before it can go out: an offline copy once
# broke Write and Draw on xenify.net only ("This site can't be reached").
if command -v chromium >/dev/null || command -v chromium-browser >/dev/null || command -v google-chrome >/dev/null; then
    scripts/check-site.py "$out"
else
    echo "No Chromium here: check the site with scripts/check-site.py before deploying it" >&2
fi
echo "Site ready in $out ($(find "$out" -type f | wc -l) files, $(du -sh "$out" | cut -f1))"
