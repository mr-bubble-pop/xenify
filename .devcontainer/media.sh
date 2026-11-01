#!/bin/sh
# Lets apps in the container use the computer's cameras (for Xenify Chat's
# calls): makes the same camera devices inside the container, readable by
# its user. The computer's own /dev is mounted read-only at /dev/host; its
# devices and permissions are left as they are. Sound comes through the
# desktop's PulseAudio/PipeWire socket (PULSE_SERVER in devcontainer.json).
for cam in /dev/host/video*; do
    [ -c "$cam" ] || continue
    name=/dev/$(basename "$cam")
    [ -e "$name" ] && continue
    major=$((0x$(stat -c %t "$cam")))
    minor=$((0x$(stat -c %T "$cam")))
    sudo mknod -m 666 "$name" c "$major" "$minor" && echo "camera: $name"
done
exit 0
