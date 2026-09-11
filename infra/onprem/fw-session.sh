#!/bin/bash
# Creates, attaches to, and destroys your session container.
#
# One instance per user, named session-<user>, living in the confined LXD
# project the multi-user daemon made for you. It persists until you destroy it
# explicitly: a dropped SSH connection must not take a running benchmark with
# it.
#
# Runs as you, not root. Your LXD access is confined to your own project, so
# there is nothing here that needs privilege.
set -o errexit
set -o nounset
set -o pipefail
# So the ERR trap in create_instance fires for failures inside the function.
set -o errtrace

IMAGE_ALIAS=firewood-session
INSTANCE="session-${USER}"
DATA_DIR="/mnt/nvme/${USER}"
READY_TIMEOUT=60

show_usage() {
    echo "Usage: $(basename "$0") [COMMAND]"
    echo ""
    echo "Commands:"
    echo "  attach     Enter your session, creating it if needed (default)"
    echo "  status     Show whether it exists, is running, and how current it is"
    echo "  destroy    Delete the instance. Your files are untouched."
    echo "  recreate   Destroy and rebuild from the current image"
    echo ""
    echo "Your home directory and $DATA_DIR are mounted from the host, so"
    echo "nothing in them is lost when the instance goes away."
}

instance_exists() {
    lxc info "$INSTANCE" > /dev/null 2>&1
}

instance_state() {
    lxc list "$INSTANCE" --format csv -c s 2>/dev/null | head -1
}

# The fingerprint the instance was created from, and the one the alias points
# at now. They diverge when a newer image has been published.
instance_image() {
    lxc config get "$INSTANCE" volatile.base_image 2>/dev/null || true
}

current_image() {
    lxc image info "$IMAGE_ALIAS" 2>/dev/null |
        awk '/^Fingerprint:/ { print $2 }' || true
}

wait_until_ready() {
    local waited=0
    until lxc exec "$INSTANCE" -- true > /dev/null 2>&1; do
        if [ "$waited" -ge "$READY_TIMEOUT" ]; then
            echo "Error: $INSTANCE did not become ready within ${READY_TIMEOUT}s" >&2
            exit 1
        fi
        sleep 1
        waited=$((waited + 1))
    done
}

create_instance() {
    if ! lxc image info "$IMAGE_ALIAS" > /dev/null 2>&1; then
        echo "Error: no image '$IMAGE_ALIAS' available." >&2
        echo "An administrator needs to build or import it." >&2
        exit 1
    fi

    echo "Creating $INSTANCE..."
    lxc launch "$IMAGE_ALIAS" "$INSTANCE"

    # A half-configured instance is worse than none: the next attach would
    # join it and find no account, no mounts, or no sudo. Remove it instead,
    # so the next attempt starts clean.
    trap 'echo "Creation failed; removing $INSTANCE" >&2
          lxc delete --force "$INSTANCE" > /dev/null 2>&1 || true' ERR

    # Sessions are restarted by their owner on login, not by the host at boot.
    lxc config set "$INSTANCE" boot.autostart false

    wait_until_ready

    # Home comes from the host so that code outlives the instance, and so that
    # ~ inside the session is the same path as outside it. The data directory
    # holds databases and build output.
    lxc config device add "$INSTANCE" home disk \
        source="$HOME" path="$HOME" > /dev/null
    lxc config device add "$INSTANCE" data disk \
        source="$DATA_DIR" path="$DATA_DIR" > /dev/null

    # The image carries no session account: create one matching this user, so
    # files written inside land owned by them outside.
    lxc exec "$INSTANCE" -- groupadd --gid "$(id -g)" --force "$(id -gn)"

    # useradd rejects names containing a dot unless --badname is given, and
    # several of these accounts are firstname.lastname.
    if ! lxc exec "$INSTANCE" -- useradd \
        --uid "$(id -u)" --gid "$(id -g)" --home-dir "$HOME" \
        --no-create-home --shell /bin/bash "$USER" 2>/dev/null; then
        lxc exec "$INSTANCE" -- useradd --badname \
            --uid "$(id -u)" --gid "$(id -g)" --home-dir "$HOME" \
            --no-create-home --shell /bin/bash "$USER"
    fi

    # Pushed as a file rather than written through `lxc exec`: /dev/stdin does
    # not resolve to the forwarded pipe inside the instance.
    sudoers="$(mktemp)"
    printf '%s ALL=(ALL) NOPASSWD:ALL\n' "$USER" > "$sudoers"
    lxc file push "$sudoers" "${INSTANCE}/etc/sudoers.d/${USER}" --mode 0440
    rm -f "$sudoers"

    trap - ERR
    echo "Created. It will persist until '$(basename "$0") destroy'."
}

warn_if_stale() {
    local was now
    was="$(instance_image)"
    now="$(current_image)"
    if [ -n "$was" ] && [ -n "$now" ] && [ "$was" != "$now" ]; then
        echo "Note: a newer session image has been published."
        echo "Run '$(basename "$0") recreate' to pick it up, once you are at a"
        echo "point where losing this instance is fine."
        echo ""
    fi
}

cmd_attach() {
    if ! instance_exists; then
        create_instance
    elif [ "$(instance_state)" != "RUNNING" ]; then
        echo "Starting $INSTANCE..."
        lxc start "$INSTANCE"
        wait_until_ready
    fi

    warn_if_stale

    # Multiple terminals can attach to the same instance; exec simply joins it.
    exec lxc exec "$INSTANCE" -- su - "$USER"
}

cmd_status() {
    if ! instance_exists; then
        echo "No session. '$(basename "$0") attach' creates one."
        return
    fi

    echo "Instance: $INSTANCE"
    echo "State:    $(instance_state)"

    local was now
    was="$(instance_image)"
    now="$(current_image)"
    if [ -n "$was" ] && [ -n "$now" ] && [ "$was" != "$now" ]; then
        echo "Image:    ${was:0:12} (newer available: ${now:0:12})"
    else
        echo "Image:    ${was:0:12} (current)"
    fi

    echo ""
    lxc config device list "$INSTANCE" | sed 's/^/Device:   /'
}

cmd_destroy() {
    if ! instance_exists; then
        echo "No session to destroy."
        return
    fi
    lxc delete --force "$INSTANCE"
    echo "Destroyed $INSTANCE. Your home directory and $DATA_DIR are untouched."
}

case "${1:-attach}" in
    attach)
        cmd_attach
        ;;
    status)
        cmd_status
        ;;
    destroy)
        cmd_destroy
        ;;
    recreate)
        cmd_destroy
        cmd_attach
        ;;
    --help | -h | help)
        show_usage
        ;;
    *)
        echo "Error: unknown command '$1'" >&2
        show_usage
        exit 1
        ;;
esac
