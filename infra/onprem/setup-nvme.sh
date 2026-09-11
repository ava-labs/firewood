#!/bin/bash
# This script sets up the NVMe scratch filesystem on the on-prem servers.
#
# It stripes every empty NVMe device into a single LVM volume group, formats it
# ext4, mounts it at /mnt/nvme, and makes it group-writable so each team member
# can keep their own working data under /mnt/nvme/$USER.
#
# It uses LVM striping rather than mdadm: LVM is already present on these
# machines, device-mapper paths are stable across reboots, and a single drive
# can later be carved off the volume group for single-disk comparisons without
# rebuilding the array. See benchmark/setup-scripts/build-environment.sh for
# the EC2 equivalent, which detects instance storage and uses mdadm.
#
# The script is idempotent -- rerunning it after a successful run makes no
# changes.
set -o errexit
set -o nounset
set -o pipefail

# Defaults
VG_NAME=firewood
LV_NAME=nvme
MOUNT_POINT=/mnt/nvme
# Default bytes-per-inode for ext4 filesystem (2MB). This suits workloads that
# create many small files, such as LevelDB under AvalancheGo. Raise it if you
# know your workload does not need that many inodes, to trade them for usable
# space.
BYTES_PER_INODE=2097152
STRIPE_SIZE=64k
GROUP_NAME=firewood
VALIDATE=1
VALIDATE_SIZE=4G
ASSUME_YES=0
DRY_RUN=0
ADD_USERS=()

show_usage() {
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Stripes all empty NVMe devices into one LVM volume and mounts it."
    echo ""
    echo "Options:"
    echo "  --vg-name NAME           Volume group name (default: firewood)"
    echo "  --lv-name NAME           Logical volume name (default: nvme)"
    echo "  --mount PATH             Mount point (default: /mnt/nvme)"
    echo "  --bytes-per-inode BYTES  ext4 bytes-per-inode (default: 2097152)"
    echo "  --stripe-size SIZE       LVM stripe size (default: 64k)"
    echo "  --group NAME             Group granted write access (default: firewood)"
    echo "  --add-user NAME          Add NAME to the group and create their"
    echo "                           directory; may be repeated"
    echo "  --no-validate            Skip the post-setup fio throughput check"
    echo "  --validate-size SIZE     Per-job fio file size (default: 4G)"
    echo "  --dry-run                Print what would be done, change nothing"
    echo "  --yes                    Do not prompt before destroying data"
    echo "  --help                   Show this help message"
}

while [[ $# -gt 0 ]]; do
    case $1 in
        --vg-name)
            VG_NAME="$2"
            shift 2
            ;;
        --lv-name)
            LV_NAME="$2"
            shift 2
            ;;
        --mount)
            MOUNT_POINT="$2"
            shift 2
            ;;
        --bytes-per-inode)
            BYTES_PER_INODE="$2"
            shift 2
            ;;
        --stripe-size)
            STRIPE_SIZE="$2"
            shift 2
            ;;
        --group)
            GROUP_NAME="$2"
            shift 2
            ;;
        --add-user)
            ADD_USERS+=("$2")
            shift 2
            ;;
        --no-validate)
            VALIDATE=0
            shift
            ;;
        --validate-size)
            VALIDATE_SIZE="$2"
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

# Echo a command, then run it unless --dry-run was given.
run() {
    echo "+ $*"
    if [ "$DRY_RUN" -eq 0 ]; then
        "$@"
    fi
}

# A device is safe to consume only if it holds no filesystem, is not mounted,
# has no partitions, and is not already an LVM physical volume.
device_is_empty() {
    local dev="$1"

    if [ -n "$(lsblk -no FSTYPE "$dev" | tr -d '[:space:]')" ]; then
        return 1
    fi
    if [ -n "$(lsblk -no MOUNTPOINT "$dev" | tr -d '[:space:]')" ]; then
        return 1
    fi
    if [ "$(lsblk -no NAME "$dev" | wc -l)" -gt 1 ]; then
        return 1
    fi
    if pvs --noheadings -o pv_name 2>/dev/null | tr -d ' ' | grep -qx "$dev"; then
        return 1
    fi
    return 0
}

# Already set up? Then there is nothing to do.
if findmnt --noheadings --mountpoint "$MOUNT_POINT" > /dev/null 2>&1; then
    echo "$MOUNT_POINT is already mounted:"
    findmnt --mountpoint "$MOUNT_POINT"
    echo ""
    echo "Nothing to do. Unmount it and remove the volume group to start over."
    SKIP_STORAGE=1
else
    SKIP_STORAGE=0
fi

if [ "$SKIP_STORAGE" -eq 0 ]; then
    if vgs --noheadings -o vg_name 2>/dev/null | tr -d ' ' | grep -qx "$VG_NAME"; then
        echo "Error: volume group '$VG_NAME' already exists but $MOUNT_POINT is" >&2
        echo "not mounted. Resolve this by hand -- refusing to guess." >&2
        exit 1
    fi

    # Collect every whole NVMe disk, then keep only the empty ones.
    mapfile -t ALL_NVME < <(lsblk -dpno NAME,TYPE |
        awk '$2 == "disk" && $1 ~ /\/nvme/ { print $1 }' | sort)

    if [ "${#ALL_NVME[@]}" -eq 0 ]; then
        echo "Error: no NVMe devices found" >&2
        exit 1
    fi

    DEVICES=()
    for dev in "${ALL_NVME[@]}"; do
        if device_is_empty "$dev"; then
            DEVICES+=("$dev")
        else
            echo "Skipping $dev: in use (filesystem, partitions, mount, or LVM PV)"
        fi
    done

    if [ "${#DEVICES[@]}" -eq 0 ]; then
        echo "Error: found ${#ALL_NVME[@]} NVMe device(s), none of them empty" >&2
        exit 1
    fi

    echo ""
    echo "Will stripe ${#DEVICES[@]} device(s) into volume group '$VG_NAME':"
    for dev in "${DEVICES[@]}"; do
        echo "  $dev  $(lsblk -dno SIZE,MODEL "$dev" | xargs)"
    done
    echo ""
    echo "ALL DATA ON THESE DEVICES WILL BE DESTROYED."
    echo ""

    if [ "$ASSUME_YES" -eq 0 ] && [ "$DRY_RUN" -eq 0 ]; then
        read -r -p "Type 'yes' to continue: " reply
        if [ "$reply" != "yes" ]; then
            echo "Aborted."
            exit 1
        fi
    fi

    for dev in "${DEVICES[@]}"; do
        run wipefs -a "$dev"
        run pvcreate "$dev"
    done

    run vgcreate "$VG_NAME" "${DEVICES[@]}"

    # A striped LV across every PV. Fall back to 99%FREE, since 100%FREE can
    # fail to round to a whole number of stripes.
    if [ "$DRY_RUN" -eq 0 ]; then
        if ! lvcreate --type striped -i "${#DEVICES[@]}" -I "$STRIPE_SIZE" \
            -l 100%FREE -n "$LV_NAME" "$VG_NAME"; then
            echo "100%FREE failed to align to stripe boundaries, retrying with 99%FREE"
            lvcreate --type striped -i "${#DEVICES[@]}" -I "$STRIPE_SIZE" \
                -l 99%FREE -n "$LV_NAME" "$VG_NAME"
        fi
    else
        echo "+ lvcreate --type striped -i ${#DEVICES[@]} -I $STRIPE_SIZE" \
            "-l 100%FREE -n $LV_NAME $VG_NAME"
    fi

    run mkfs.ext4 -E nodiscard -i "$BYTES_PER_INODE" -L "$LV_NAME" "$LV_PATH"
    run mkdir -p "$MOUNT_POINT"

    # Mount by UUID so the entry cannot be broken by device renaming, and with
    # nofail so a bad disk leaves the machine reachable over SSH instead of
    # dropping it to an emergency console. These machines are remote-only apart
    # from the PiKVM, and physical access is slow.
    if [ "$DRY_RUN" -eq 0 ]; then
        FS_UUID="$(blkid -s UUID -o value "$LV_PATH")"
        if grep -q "[[:space:]]${MOUNT_POINT}[[:space:]]" /etc/fstab; then
            echo "An fstab entry for $MOUNT_POINT already exists, leaving it alone"
        else
            echo "UUID=$FS_UUID $MOUNT_POINT ext4 defaults,noatime,nofail 0 2" >> /etc/fstab
        fi
        systemctl daemon-reload
        mount "$MOUNT_POINT"
    else
        echo "+ append UUID=<uuid> $MOUNT_POINT ext4 defaults,noatime,nofail 0 2 to /etc/fstab"
        echo "+ systemctl daemon-reload"
        echo "+ mount $MOUNT_POINT"
    fi
fi

# Shared write access. The setgid bit makes new subdirectories inherit the
# group, so one member's files stay writable by the others.
run groupadd -f "$GROUP_NAME"
run chgrp "$GROUP_NAME" "$MOUNT_POINT"
run chmod 2775 "$MOUNT_POINT"

for user in ${ADD_USERS+"${ADD_USERS[@]}"}; do
    if ! id -u "$user" > /dev/null 2>&1; then
        echo "Warning: user '$user' does not exist, skipping" >&2
        continue
    fi
    run usermod -aG "$GROUP_NAME" "$user"
    run mkdir -p "$MOUNT_POINT/$user/firewood"
    run chown -R "$user:$user" "$MOUNT_POINT/$user"
done

# Confirm the geometry is what was asked for before anyone trusts a benchmark
# number taken from this filesystem.
if [ "$DRY_RUN" -eq 0 ]; then
    echo ""
    echo "=== Logical volume ==="
    lvs -o lv_name,lv_size,stripes,stripe_size,devices "$VG_NAME"
    echo ""
    echo "=== Filesystem ==="
    df -hT "$MOUNT_POINT"
fi

if [ "$VALIDATE" -eq 1 ] && [ "$DRY_RUN" -eq 0 ]; then
    if ! dpkg -s fio > /dev/null 2>&1; then
        apt-get install -y fio
    fi

    FIO_DIR="$MOUNT_POINT/.fio-validate"
    mkdir -p "$FIO_DIR"
    trap 'rm -rf "$FIO_DIR"' EXIT

    NUM_JOBS="$(lvs --noheadings -o stripes "$VG_NAME/$LV_NAME" | tr -d ' ')"

    echo ""
    echo "=== Sequential read throughput ==="
    echo "Note: on consumer boards most M.2 slots share a chipset uplink of"
    echo "roughly 8 GB/s, so throughput will not scale linearly with drive"
    echo "count. Record this number as the ceiling for this machine."
    fio --name=seq --directory="$FIO_DIR" --rw=read --bs=1M \
        --size="$VALIDATE_SIZE" --numjobs="$NUM_JOBS" --iodepth=32 \
        --ioengine=libaio --direct=1 --group_reporting
fi

echo ""
echo "Done. Each team member should run this once, as themselves:"
echo ""
echo "  mkdir -p $MOUNT_POINT/\$USER/firewood"
echo "  ln -s $MOUNT_POINT/\$USER/firewood ~/firewood"
echo ""
echo "Then reboot and confirm $MOUNT_POINT comes back before relying on it."
