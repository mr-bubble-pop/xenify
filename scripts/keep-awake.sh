#!/usr/bin/env bash
# Keeps the computer from sleeping while a long job (model training or
# scoring) runs, and lets it sleep normally the rest of the time.
#
# Run this on the host (not in the dev container) and leave it running, e.g.
# from XFCE's Session and Startup › Application Autostart. Jobs write a "keep
# awake until" Unix time to target/KEEP-AWAKE; while that time is in the
# future, sleep is blocked (systemd-inhibit). The time runs out by itself, so a
# forgotten file never keeps the computer awake for long.
#
#   echo $(( $(date +%s) + 3*3600 )) > target/KEEP-AWAKE   # 3 hours from now
#   rm target/KEEP-AWAKE                                     # done early
set -u
flag="$(cd "$(dirname "$0")/.." && pwd)/target/KEEP-AWAKE"
while true; do
    until_time=$(tr -dc '0-9' < "$flag" 2>/dev/null)
    if [ "$(date +%s)" -lt "${until_time:-0}" ]; then
        systemd-inhibit --what=sleep:idle --who="Xenify" --why="A long job is running" sleep 60
    else
        sleep 60
    fi
done
