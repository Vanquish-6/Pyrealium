#!/bin/sh
# Read-only capacity gate. Never cleans disk, reloads services or opens ports.
set -eu
target=${1:-/}
min_free_kib=12582912 # 12 GiB before copying the matching game assets.
min_available_kib=4194304 # 4 GiB at the instant of inspection.
failed=0
printf 'Architecture: '; uname -m
printf 'Logical CPUs: '; getconf _NPROCESSORS_ONLN
uptime
free -m
df -h "$target"
free_kib=$(df -Pk "$target" | awk 'NR==2 {print $4}')
available_kib=$(awk '/^MemAvailable:/ {print $2}' /proc/meminfo)
if [ "$free_kib" -lt "$min_free_kib" ]; then
    echo 'BLOCKED: need at least 12 GiB free before installing AC2 assets.' >&2
    failed=1
fi
if [ "$available_kib" -lt "$min_available_kib" ]; then
    echo 'BLOCKED: less than 4 GiB RAM currently available.' >&2
    failed=1
fi
for unit in aetherium.service aetherium-pk.service nginx.service; do
    systemctl show "$unit" -p Id -p ActiveState -p MainPID -p NRestarts
    if ! systemctl is-active --quiet "$unit"; then
        echo "BLOCKED: existing service $unit is not active; investigate first." >&2
        failed=1
    fi
done
listeners=$(ss -H -lntu)
if printf '%s\n' "$listeners" | awk '{print $5}' | grep -Eq ':(19370|19371|19400)$'; then
    echo 'BLOCKED: an AC2 port is already occupied; inspect its owner.' >&2
    failed=1
fi
printf '%s\n' "$listeners"
if [ "$failed" -ne 0 ]; then exit 1; fi
echo 'Capacity snapshot passed. Peak-load capacity and WAN gameplay remain unverified.'
