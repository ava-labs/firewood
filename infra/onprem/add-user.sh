#!/bin/bash
# Creates a team member's account on this machine and runs every setup step
# they need.
#
# Accounts are created by hand on each machine rather than through Okta; see
# ADMINISTRATION.md for why. This script is the one place that knows the full
# list of steps, so adding a step here covers everyone added from then on.
#
# Run it once per machine for each person.
#
# Login uses Cloudflare short-lived certificates, so no password or SSH key is
# created. The account is left with password login disabled.
set -o errexit
set -o nounset
set -o pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Defaults
GROUP_NAME=firewood
MOUNT_POINT=/mnt/nvme
FULL_NAME=""
GRANT_SUDO=0
DRY_RUN=0
USERNAME=""

show_usage() {
    echo "Usage: $0 [OPTIONS] USERNAME"
    echo ""
    echo "Creates USERNAME and runs every per-user setup step."
    echo ""
    echo "Options:"
    echo "  --full-name NAME         Real name recorded in /etc/passwd"
    echo "  --sudo                   Add the account to the sudo group"
    echo "  --group NAME             Shared group to join (default: firewood)"
    echo "  --mount PATH             NVMe mount point (default: /mnt/nvme)"
    echo "  --dry-run                Print what would be done, change nothing"
    echo "  --help                   Show this help message"
}

while [[ $# -gt 0 ]]; do
    case $1 in
        --full-name)
            FULL_NAME="$2"
            shift 2
            ;;
        --sudo)
            GRANT_SUDO=1
            shift
            ;;
        --group)
            GROUP_NAME="$2"
            shift 2
            ;;
        --mount)
            MOUNT_POINT="$2"
            shift 2
            ;;
        --dry-run)
            DRY_RUN=1
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

# Echo a command, then run it unless --dry-run was given.
run() {
    echo "+ $*"
    if [ "$DRY_RUN" -eq 0 ]; then
        "$@"
    fi
}

# --- Account ---------------------------------------------------------------

if id -u "$USERNAME" > /dev/null 2>&1; then
    echo "User '$USERNAME' already exists, leaving the account alone"
else
    # Password login stays disabled: authentication is handled by Cloudflare
    # short-lived certificates before the connection reaches this machine.
    run adduser --disabled-password --gecos "$FULL_NAME" "$USERNAME"
fi

if [ "$GRANT_SUDO" -eq 1 ]; then
    run usermod -aG sudo "$USERNAME"
    echo ""
    echo "Note: '$USERNAME' is in the sudo group but has no password, so sudo"
    echo "will reject them. Either set one with 'passwd $USERNAME' or add a"
    echo "NOPASSWD rule under /etc/sudoers.d/."
    echo ""
fi

# --- Storage ---------------------------------------------------------------

# setup-nvme.sh owns the group membership, the per-user directory under the
# NVMe mount, and the ~/firewood link. It skips the storage work on a machine
# that is already configured.
run bash "$SCRIPT_DIR/setup-nvme.sh" \
    --add-user "$USERNAME" \
    --group "$GROUP_NAME" \
    --mount "$MOUNT_POINT" \
    --no-validate \
    --yes

# --- Further steps ---------------------------------------------------------

# Add anything else a new account needs here, so that one script remains the
# complete answer. Candidates: session container profile, shell defaults,
# per-user metrics scrape configuration.

echo ""
echo "Done. '$USERNAME' can now reach this machine over SSH, and their group"
echo "membership takes effect at their next login."
echo ""
echo "Repeat this on the other machine: accounts are per-host."
