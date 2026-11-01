#!/usr/bin/env bash
# Keeps target/ from filling the disk: removes build output nobody has used
# for a while. It's only cache (cargo makes it again when it's needed).
#
#   scripts/clean-target.sh              once
#   scripts/clean-target.sh --every 6h   again every 6 hours (the dev
#                                        container starts it this way)
#
# What goes:
#   - incremental compilation caches untouched for 12 hours
#   - compiled crates, test programs and build-script output (deps/, build/)
#     untouched for 3 days, in every profile and target
# Never touched: anything else in target/ (the site and app builds,
# worktrees). The grammar checker's training data is in data/, not here.
set -uo pipefail
cd "$(dirname "$0")/.."

clean() {
    local before after
    before=$(df -k --output=avail . | tail -n 1)
    # Profiles at target/<profile>/ and target/<triple>/<profile>/.
    for dir in target/*/ target/*/*/; do
        case "$dir" in
            target/tmp/*) continue ;;
        esac
        [ -d "$dir/incremental" ] && find "$dir/incremental" -mindepth 1 -maxdepth 1 -mmin +720 -exec rm -rf {} + 2>/dev/null
        for sub in deps build; do
            [ -d "$dir/$sub" ] || continue
            # Only folders that look like cargo's (it keeps .fingerprint beside them).
            [ -d "$dir/.fingerprint" ] || continue
            find "$dir/$sub" -mindepth 1 -maxdepth 1 -mtime +3 -exec rm -rf {} + 2>/dev/null
        done
    done
    after=$(df -k --output=avail . | tail -n 1)
    echo "clean-target: freed $(( (after - before) / 1024 )) MB ($(( after / 1024 / 1024 )) GB free)"
}

case "${1:-}" in
    --every)
        every=${2:-6h}
        while true; do clean; sleep "$every"; done ;;
    "") clean ;;
    *) echo "usage: $0 [--every 6h]" >&2; exit 1 ;;
esac
