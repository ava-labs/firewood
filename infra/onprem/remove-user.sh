#!/bin/bash
# Removes a team member's account and their session infrastructure.
#
# By default their files are kept: the home directory and the data directory
# under the NVMe array are left in place, owned by the now-unused uid. Someone
# leaving often has work others still need, and deleting terabytes of it is not
# reversible. Pass --purge to remove those too, deliberately.
#
# Run it once per machine, as accounts are per host.
set -o errexit
set -o nounset
set -o pipefail

MOUNT_POINT=/mnt/nvme
PURGE=0
DRY_RUN=0
ASSUME_YES=0
USERNAME=""

show_usage() {
    echo "Usage: $0 [OPTIONS] USERNAME"
    echo ""
    echo "Removes USERNAME's account, session and LXD project."
    echo ""
    echo "Options:"
    echo "  --purge                  Also delete their home directory and"
    echo "                           $MOUNT_POINT/USERNAME. Not reversible."
    echo "  --mount PATH             NVMe mount point (default: /mnt/nvme)"
    echo "  --dry-run                Print what would be done, change nothing"
    echo "  --yes                    Do not prompt"
    echo "  --help                   Show this help message"
}

while [[ $# -gt 0 ]]; do
    case $1 in
        --purge)
            PURGE=1
            shift
            ;;
        --mount)
            MOUNT_POINT="$2"
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
        -*)
            echo "Error: Unknown option $1" >&2
            show_usage
            exit 1
            ;;
        *)
            if [ -n "$USERNAME" ]; then
                echo "Error: more than one username given" >&2
                exit 1
            fi
            USERNAME="$1"
            shift
            ;;
    esac
done

if [ -z "$USERNAME" ]; then
    echo "Error: no username given" >&2
    show_usage
    exit 1
fi

# Checked after argument parsing so --help works without sudo.
if [ "$EUID" -ne 0 ]; then
    echo "This script must be run as root" >&2
    exit 1
fi

if ! id -u "$USERNAME" > /dev/null 2>&1; then
    echo "Error: no such user '$USERNAME'" >&2
    exit 1
fi

run() {
    echo "+ $*"
    if [ "$DRY_RUN" -eq 0 ]; then
        "$@"
    fi
}

# Captured before the account goes: the LXD project is named after the uid.
USER_UID="$(id -u "$USERNAME")"
USER_HOME="$(getent passwd "$USERNAME" | cut -d: -f6)"
PROJECT="user-${USER_UID}"
BRIDGE="lxdbr-${USER_UID}"
DATA_DIR="$MOUNT_POINT/$USERNAME"

if who | awk '{print $1}' | grep -qx "$USERNAME"; then
    echo "Error: '$USERNAME' is logged in. Ask them to log out first." >&2
    exit 1
fi

echo ""
echo "Will remove account '$USERNAME' (uid $USER_UID) from $(hostname):"
echo "  LXD project $PROJECT and any instances in it"
echo "  network $BRIDGE"
echo "  the account itself"
if [ "$PURGE" -eq 1 ]; then
    echo ""
    echo "  AND PERMANENTLY DELETE:"
    echo "    $USER_HOME"
    echo "    $DATA_DIR ($(du -sh "$DATA_DIR" 2>/dev/null | cut -f1 || echo unknown))"
else
    echo ""
    echo "Keeping their files. Pass --purge to delete them:"
    echo "  $USER_HOME"
    echo "  $DATA_DIR"
fi
echo ""

if [ "$ASSUME_YES" -eq 0 ] && [ "$DRY_RUN" -eq 0 ]; then
    read -r -p "Type 'yes' to continue: " reply
    if [ "$reply" != "yes" ]; then
        echo "Aborted."
        exit 1
    fi
fi

# --- Session and LXD project -----------------------------------------------

if command -v lxc > /dev/null 2>&1; then
    if lxc project list --format csv 2>/dev/null | cut -d, -f1 |
        grep -qx "$PROJECT"; then

        while read -r instance; do
            [ -n "$instance" ] || continue
            run lxc delete --force "$instance" --project "$PROJECT"
        done < <(lxc list --project "$PROJECT" --format csv -c n 2>/dev/null)

        # A profile holding a root disk counts as content, so the project will
        # not delete until its devices are gone.
        while read -r device; do
            [ -n "$device" ] || continue
            run lxc profile device remove default "$device" --project "$PROJECT"
        done < <(lxc profile device list default --project "$PROJECT" 2>/dev/null)

        run lxc project delete "$PROJECT"
    else
        echo "No LXD project $PROJECT"
    fi

    # Created by the multi-user daemon alongside the project. Deleted after it,
    # since the project's profile referenced it.
    if lxc network list --format csv 2>/dev/null | cut -d, -f1 |
        grep -qx "$BRIDGE"; then
        run lxc network delete "$BRIDGE"
    fi
else
    echo "Warning: lxc not found, skipping LXD cleanup" >&2
fi

# --- Account ----------------------------------------------------------------

if [ "$PURGE" -eq 1 ]; then
    run userdel --remove "$USERNAME"
    if [ -d "$DATA_DIR" ]; then
        run rm -rf "$DATA_DIR"
    fi
else
    run userdel "$USERNAME"
    echo ""
    echo "Kept $USER_HOME and $DATA_DIR, now owned by uid $USER_UID with no"
    echo "account. Reassign or delete them when you know what is in them:"
    echo ""
    echo "  chown -R <someone> $DATA_DIR"
    echo "  rm -rf $USER_HOME $DATA_DIR"
fi

echo ""
echo "Done. Repeat on the other machine: accounts are per host."
