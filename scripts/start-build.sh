#!/usr/bin/env bash
# Starts a build on the public mr-bubble-pop/xenify-releases repository,
# where Actions minutes are free, by pushing a tag there. Its workflows
# (.github/workflows/release.yml and site.yml in that repository) build
# what's on this repository's main branch on GitHub, so push first.
#
#   scripts/start-build.sh release [nightly|preview|release]   Linux packages, signed list, Hub
#   scripts/start-build.sh site                                xenify.net
#
# Pushes with the deploy key .ssh/claude_releases_ed25519 (or set
# XENIFY_RELEASES_SSH to another ssh command). Watch it on
# https://github.com/mr-bubble-pop/xenify-releases/actions
set -euo pipefail
cd "$(dirname "$0")/.."
what=${1:?release or site}
remote=git@github.com:mr-bubble-pop/xenify-releases.git
ssh_cmd=${XENIFY_RELEASES_SSH:-"ssh -i $PWD/.ssh/claude_releases_ed25519 -o IdentitiesOnly=yes"}
case "$what" in
    release) channel=${2:-nightly}
        case "$channel" in nightly|preview|release) ;; *) echo "channel: nightly, preview or release" >&2; exit 1 ;; esac
        tag="build-$channel-$(date -u +%Y%m%d-%H%M%S)" ;;
    site) tag="site-$(date -u +%Y%m%d-%H%M%S)" ;;
    *) echo "release or site" >&2; exit 1 ;;
esac
# Unpushed work wouldn't be in the build.
git fetch -q origin main
if [ "$(git rev-parse HEAD)" != "$(git rev-parse origin/main)" ]; then
    echo "note: this checkout isn't what's on GitHub's main; the build uses origin/main ($(git rev-parse --short origin/main))" >&2
fi
# The tag goes on the releases repository's own main (its README commit).
head=$(GIT_SSH_COMMAND="$ssh_cmd" git ls-remote "$remote" refs/heads/main | cut -f1)
[ -n "$head" ] || { echo "can't reach $remote" >&2; exit 1; }
# (Pushing a commit needs it here.)
git fetch -q https://github.com/mr-bubble-pop/xenify-releases.git main
GIT_SSH_COMMAND="$ssh_cmd" git push -q "$remote" "$head:refs/tags/$tag"
echo "started $tag: https://github.com/mr-bubble-pop/xenify-releases/actions"
