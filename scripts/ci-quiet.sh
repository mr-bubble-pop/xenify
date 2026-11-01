#!/usr/bin/env bash
# Runs a build step for the public xenify-releases repository's workflows,
# whose logs anyone can read: the compiler quotes lines of code in its
# warnings and errors, so the output goes to a file instead. On success it
# says so; on failure it prints the error messages and the last lines,
# leaving out the quoted code (lines like " 12 | let x = …", "  --> src/…",
# "   |      ^^^"). Rebuild here to see the whole message.
#
#   scripts/ci-quiet.sh <command> [args…]
set -uo pipefail
log=$(mktemp)
start=$(date +%s)
"$@" >"$log" 2>&1
status=$?
took=$(($(date +%s) - start))
if [ $status -eq 0 ]; then
    echo "ok after ${took}s: $*"
    rm -f "$log"
    exit 0
fi
# Code quotes: "NN | …", "   | …", "  --> file", "...", "  = note" stays.
hide='^[[:space:]]*([0-9]+[[:space:]]*)?\|($|[[:space:]])|^[[:space:]]*-->|^[[:space:]]*:::|^\.\.\.$|^[[:space:]]*[+-][[:space:]]'
echo "failed (exit $status) after ${took}s: $*"
echo "--- errors"
grep -E '^error' "$log" | grep -vE "$hide" | sort -u | head -40
echo "--- last lines"
grep -vE "$hide" "$log" | grep -vE '^(warning|help|[[:space:]]*= )' | tail -25
rm -f "$log"
exit $status
