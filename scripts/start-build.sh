#!/usr/bin/env bash
# Starts a build on GitHub by pushing a tag to origin's main: the workflows
# in .github/workflows (release.yml and site.yml) build that commit, so push
# your work first.
#
#   scripts/start-build.sh release [nightly|preview|release]   packages, signed list, Hub
#   scripts/start-build.sh site                                xenify.net
#
# Watch it on https://github.com/mr-bubble-pop/xenify/actions
set -euo pipefail
cd "$(dirname "$0")/.."
what=${1:?release or site}
case "$what" in
    release) channel=${2:-nightly}
        case "$channel" in nightly|preview|release) ;; *) echo "channel: nightly, preview or release" >&2; exit 1 ;; esac
        tag="build-$channel-$(date -u +%Y%m%d-%H%M%S)" ;;
    site) tag="site-$(date -u +%Y%m%d-%H%M%S)" ;;
    *) echo "release or site" >&2; exit 1 ;;
esac
git fetch -q origin main
if [ "$(git rev-parse HEAD)" != "$(git rev-parse origin/main)" ]; then
    echo "note: this checkout isn't what's on GitHub's main; the build uses origin/main ($(git rev-parse --short origin/main))" >&2
fi
git push -q origin "$(git rev-parse origin/main):refs/tags/$tag"
echo "started $tag: https://github.com/mr-bubble-pop/xenify/actions"
