#!/bin/sh
# Copy the host's DNS config into running xenify devcontainers.
#
# Docker snapshots the host's /etc/resolv.conf when a container is created, so
# toggling a VPN on the host (which swaps DNS servers) leaves the container
# pointing at a server that no longer answers. Run this on the HOST after the
# DNS changes, or install it as a NetworkManager dispatcher hook so it runs
# automatically on every VPN up/down:
#
#   sudo install -m 755 .devcontainer/sync-dns.sh /etc/NetworkManager/dispatcher.d/90-devcontainer-dns
#
# NetworkManager runs dispatcher hooks as root with the interface and event as
# arguments; we resync on any event that can change DNS.

case "$2" in
    ""|up|down|vpn-up|vpn-down|dhcp4-change|dhcp6-change|connectivity-change) ;;
    *) exit 0 ;;
esac

# If the host runs systemd-resolved, /etc/resolv.conf points at the 127.0.0.53
# stub, which is unreachable from a container; use the real upstream list.
src=/etc/resolv.conf
if grep -q '^nameserver 127\.' "$src" && [ -f /run/systemd/resolve/resolv.conf ]; then
    src=/run/systemd/resolve/resolv.conf
fi

for id in $(docker ps -q --filter label=devcontainer.local_folder); do
    # Only this project's container(s).
    case "$(docker inspect -f '{{index .Config.Labels "devcontainer.local_folder"}}' "$id")" in
        */xenify) docker exec -i -u root "$id" sh -c 'cat > /etc/resolv.conf' < "$src" ;;
    esac
done
