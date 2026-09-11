#!/bin/bash
# Creates a team member's account on this machine and runs every setup step
# they need.
#
# Accounts are created by hand on each machine rather than through Okta; see
# SETUP.md for why. This script is the one place that knows the full
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
    #
    # adduser enforces NAME_REGEX from /etc/adduser.conf, which excludes dots,
    # and these accounts are firstname.lastname. The flag for allowing them was
    # renamed, so try both.
    if ! run adduser --disabled-password --gecos "$FULL_NAME" "$USERNAME"; then
        if ! run adduser --allow-bad-names --disabled-password \
            --gecos "$FULL_NAME" "$USERNAME"; then
            run adduser --force-badname --disabled-password \
                --gecos "$FULL_NAME" "$USERNAME"
        fi
    fi
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

# The array must already exist. setup-nvme.sh is called below only for its
# per-user work, but it is also capable of creating the array, and --yes would
# suppress the confirmation for that. Refusing here keeps account creation from
# formatting disks as a side effect.
if ! findmnt --noheadings --mountpoint "$MOUNT_POINT" > /dev/null 2>&1; then
    echo "Error: $MOUNT_POINT is not mounted." >&2
    echo "Run setup-nvme.sh first; see SETUP.md." >&2
    exit 1
fi

# setup-nvme.sh owns the group membership, the per-user directory under the
# NVMe mount, and the ~/firewood link. It skips the storage work on a machine
# that is already configured.
run bash "$SCRIPT_DIR/setup-nvme.sh" \
    --add-user "$USERNAME" \
    --group "$GROUP_NAME" \
    --mount "$MOUNT_POINT" \
    --no-validate \
    --yes

# --- Session tool ----------------------------------------------------------

# Installed rather than run from a checkout: a script under one person's home
# directory is not readable by other accounts, and every user would otherwise
# need their own clone kept up to date. Refreshed on every run, so this is not
# a separate step anyone can forget.
FW_SESSION_SRC="$SCRIPT_DIR/fw-session.sh"
FW_SESSION_DST=/usr/local/bin/fw-session
if [ -f "$FW_SESSION_SRC" ]; then
    if ! cmp -s "$FW_SESSION_SRC" "$FW_SESSION_DST"; then
        echo "Installing $FW_SESSION_DST"
        run install -m 0755 "$FW_SESSION_SRC" "$FW_SESSION_DST"
    fi
else
    echo "Warning: $FW_SESSION_SRC not found, skipping fw-session install" >&2
fi

# Attaches interactive SSH logins to a session. Installed alongside
# fw-session so the two never disagree about whether sessions are in use.
PROFILE_SRC="$SCRIPT_DIR/profile-fw-session.sh"
PROFILE_DST=/etc/profile.d/fw-session.sh
if [ -f "$PROFILE_SRC" ]; then
    if ! cmp -s "$PROFILE_SRC" "$PROFILE_DST"; then
        echo "Installing $PROFILE_DST"
        run install -m 0644 "$PROFILE_SRC" "$PROFILE_DST"
    fi
else
    echo "Warning: $PROFILE_SRC not found, skipping login hook" >&2
fi

# --- LXD project -----------------------------------------------------------

# LXD's multi-user daemon creates a confined project the first time a member of
# the group runs any lxc command, naming it after their uid. Rather than wait
# for the user's first login, trigger it here as them, then configure it.
if command -v lxc > /dev/null 2>&1; then
    USER_UID="$(id -u "$USERNAME")"
    PROJECT="user-${USER_UID}"

    run sudo -u "$USERNAME" -- lxc project list > /dev/null 2>&1 || true

    if lxc project list --format csv 2>/dev/null | cut -d, -f1 | grep -qx "$PROJECT"; then
        # Confined projects get their own image store by default, which would
        # mean a private copy of the multi-gigabyte session image per user.
        # Share the default project's images instead.
        #
        # This has to happen before the user caches any image of their own:
        # LXD refuses to disable the feature on a project that already holds
        # images. Triggering project creation with `lxc project list` above
        # rather than by launching anything keeps the project empty here.
        if ! run lxc project set "$PROJECT" features.images false; then
            echo "" >&2
            echo "Could not share images into '$PROJECT'. If it already holds" >&2
            echo "cached images, remove them and retry:" >&2
            echo "" >&2
            echo "  lxc image list --project $PROJECT --format csv -c f |" >&2
            echo "    xargs -r -n1 lxc image delete --project $PROJECT" >&2
            echo "  lxc project set $PROJECT features.images false" >&2
            echo "" >&2
        fi

        # Permit disk devices, but only with sources under this user's own
        # home and data directories, which is what fw-session mounts. `allow`
        # with an empty paths list would permit any host path, which is host
        # root by another route.
        user_home="$(getent passwd "$USERNAME" | cut -d: -f6)"
        run lxc project set "$PROJECT" restricted.devices.disk allow
        run lxc project set "$PROJECT" restricted.devices.disk.paths \
            "${user_home},${MOUNT_POINT}/${USERNAME}"

        # The daemon puts a root disk and a nic in the project's own default
        # profile when it creates it. Repair them if they are missing: without
        # a root device, launching anything fails with "No root device could
        # be found", which does not point anywhere useful.
        if ! lxc profile device list default --project "$PROJECT" 2>/dev/null |
            grep -qx root; then
            echo "Restoring the root disk device in '$PROJECT'"
            run lxc profile device add default root disk path=/ pool=default \
                --project "$PROJECT"
        fi

        if ! lxc profile device list default --project "$PROJECT" 2>/dev/null |
            grep -qx eth0; then
            # The daemon makes a per-user bridge and names it in
            # restricted.networks.access.
            net="$(lxc project get "$PROJECT" restricted.networks.access 2>/dev/null |
                tr -d ' ')"
            if [ -n "$net" ]; then
                echo "Restoring the network device in '$PROJECT'"
                run lxc profile device add default eth0 nic \
                    network="$net" name=eth0 --project "$PROJECT"
            else
                echo "Warning: no nic in '$PROJECT' and no network to attach" >&2
            fi
        fi

        if [ "$DRY_RUN" -eq 0 ]; then
            echo ""
            lxc project show "$PROJECT" | grep -E 'features.images|restricted.devices.disk'
            lxc profile device list default --project "$PROJECT" | sed 's/^/Device: /'
        fi
    else
        echo "Warning: project '$PROJECT' was not created. Check that" >&2
        echo "'snap get lxd daemon.user.group' names a group '$USERNAME' is in," >&2
        echo "then rerun this script." >&2
    fi
else
    echo "Warning: lxc not found, skipping LXD project setup" >&2
fi

# --- Further steps ---------------------------------------------------------

# Add anything else a new account needs here, so that one script remains the
# complete answer. Candidates: session container profile, shell defaults,
# per-user metrics scrape configuration.

echo ""
echo "Done. '$USERNAME' can now reach this machine over SSH, start a session"
echo "with 'fw-session', and their group membership takes effect at their next"
echo "login."
echo ""
echo "Repeat this on the other machine: accounts are per-host."
