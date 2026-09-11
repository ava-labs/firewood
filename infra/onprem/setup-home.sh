#!/bin/bash
# Moves /home onto its own logical volume.
#
# Home directories otherwise share the 98 GB root filesystem, which a couple of
# Rust target directories can fill. The root volume group has several terabytes
# unallocated, so home gets its own volume out of that.
#
# Source code lives in home, on the SATA system disk. Build artefacts and
# databases belong on the NVMe array instead; see ADMINISTRATION.md.
#
# The existing /home content is copied, not moved. The originals stay in place,
# hidden underneath the new mount, until someone reclaims the space
# deliberately.
set -o errexit
set -o nounset
set -o pipefail

VG_NAME=ubuntu-vg
LV_NAME=home
LV_SIZE=1T
MOUNT_POINT=/home
STAGING=/mnt/home-staging
DRY_RUN=0
ASSUME_YES=0

show_usage() {
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Moves /home onto its own logical volume."
    echo ""
    echo "Options:"
    echo "  --size SIZE              Volume size (default: 1T)"
    echo "  --vg-name NAME           Volume group to carve it from (default: ubuntu-vg)"
    echo "  --lv-name NAME           Logical volume name (default: home)"
    echo "  --dry-run                Print what would be done, change nothing"
    echo "  --yes                    Do not prompt before making changes"
    echo "  --help                   Show this help message"
}

while [[ $# -gt 0 ]]; do
    case $1 in
        --size)
            LV_SIZE="$2"
            shift 2
            ;;
        --vg-name)
            VG_NAME="$2"
            shift 2
            ;;
        --lv-name)
            LV_NAME="$2"
            shift 2
            ;;
        --dry-run)
            DRY_RUN=1
            shift
            ;;
        --yes)
            ASSUME_YES=1
            shift
            ;;
        --help)
            show_usage
            exit 0
            ;;
        *)
            echo "Error: Unknown option $1" >&2
            show_usage
            exit 1
            ;;
    esac
done

# Checked after argument parsing so --help works without sudo.
if [ "$EUID" -ne 0 ]; then
    echo "This script must be run as root" >&2
    exit 1
fi

LV_PATH="/dev/${VG_NAME}/${LV_NAME}"

run() {
    echo "+ $*"
    if [ "$DRY_RUN" -eq 0 ]; then
        "$@"
    fi
}

if findmnt --noheadings --mountpoint "$MOUNT_POINT" > /dev/null 2>&1; then
    echo "$MOUNT_POINT is already a separate mount:"
    findmnt --mountpoint "$MOUNT_POINT"
    echo ""
    echo "Nothing to do."
    exit 0
fi

if ! vgs --noheadings -o vg_name 2>/dev/null | tr -d ' ' | grep -qx "$VG_NAME"; then
    echo "Error: volume group '$VG_NAME' not found" >&2
    exit 1
fi

# Copying home while someone is writing to it loses their work. Refuse if
# anyone else is logged in, but not for the operator running this: they are
# necessarily logged in themselves, and requiring a root console login would
# mean a trip to the PiKVM for a routine change.
OPERATOR="${SUDO_USER:-root}"
LOGGED_IN="$(who | awk '{print $1}' | sort -u |
    grep -vx -e root -e "$OPERATOR" | tr '\n' ' ' || true)"
if [ -n "$LOGGED_IN" ]; then
    echo "Error: these users are logged in: ${LOGGED_IN% }" >&2
    echo "Run this with the machine quiet." >&2
    exit 1
fi

# The operator's own shell is tolerated, so their home is copied while they
# are in it. rsync runs again immediately before the swap to pick up anything
# that changed, and the working directory should be outside $MOUNT_POINT.
case "$PWD" in
    "$MOUNT_POINT"/*)
        echo "Error: run this from outside $MOUNT_POINT (try 'cd /')" >&2
        exit 1
        ;;
esac

CURRENT_SIZE="$(du -sh "$MOUNT_POINT" 2>/dev/null | cut -f1)"
echo ""
echo "Will create a ${LV_SIZE} volume '${LV_NAME}' in '${VG_NAME}' and copy"
echo "$MOUNT_POINT (currently ${CURRENT_SIZE:-unknown}) onto it."
echo ""
vgs --units g -o vg_name,vg_size,vg_free "$VG_NAME"
echo ""

if [ "$ASSUME_YES" -eq 0 ] && [ "$DRY_RUN" -eq 0 ]; then
    read -r -p "Type 'yes' to continue: " reply
    if [ "$reply" != "yes" ]; then
        echo "Aborted."
        exit 1
    fi
fi

run lvcreate -L "$LV_SIZE" -n "$LV_NAME" "$VG_NAME"
run mkfs.ext4 -L "$LV_NAME" "$LV_PATH"

# Copy through a staging mount, so the originals stay readable until the very
# end and the operation can be abandoned at any point before the swap.
run mkdir -p "$STAGING"
run mount "$LV_PATH" "$STAGING"
run rsync -aHAX --info=progress2 "$MOUNT_POINT/" "$STAGING/"

if [ "$DRY_RUN" -eq 0 ]; then
    echo ""
    echo "Comparing the copy against the original..."
    # mkfs creates lost+found on the new volume; the old /home is a plain
    # directory and has none, so comparing without excluding it always differs.
    if ! diff -rq --exclude=lost+found "$MOUNT_POINT" "$STAGING" \
        > /tmp/home-copy-diff 2>&1; then
        echo "Error: copy differs from the original. Nothing has been swapped." >&2
        echo "See /tmp/home-copy-diff. The new volume is mounted at $STAGING." >&2
        exit 1
    fi
    echo "Copy verified."
fi

# Catch anything written during the copy, including by the operator's own
# shell. Cheap: rsync only transfers the delta.
run rsync -aHAX --delete "$MOUNT_POINT/" "$STAGING/"

run umount "$STAGING"
run rmdir "$STAGING"

# nofail for the same reason as the NVMe mount: a failed mount must not drop
# the machine to an emergency console, since physical access is slow.
if [ "$DRY_RUN" -eq 0 ]; then
    FS_UUID="$(blkid -s UUID -o value "$LV_PATH")"
    if grep -q "[[:space:]]${MOUNT_POINT}[[:space:]]" /etc/fstab; then
        echo "An fstab entry for $MOUNT_POINT already exists, leaving it alone"
    else
        echo "UUID=$FS_UUID $MOUNT_POINT ext4 defaults,nofail 0 2" >> /etc/fstab
    fi
    systemctl daemon-reload
    mount "$MOUNT_POINT"
else
    echo "+ append UUID=<uuid> $MOUNT_POINT ext4 defaults,nofail 0 2 to /etc/fstab"
    echo "+ systemctl daemon-reload"
    echo "+ mount $MOUNT_POINT"
fi

if [ "$DRY_RUN" -eq 0 ]; then
    echo ""
    df -hT "$MOUNT_POINT"
    echo ""
    echo "Done. The previous contents of $MOUNT_POINT are still on the root"
    echo "filesystem, hidden under the new mount. Reclaim that space once you"
    echo "are satisfied, by booting with $MOUNT_POINT unmounted."
    echo ""
    echo "Reboot and confirm $MOUNT_POINT returns before relying on it."
fi
