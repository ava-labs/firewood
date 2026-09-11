# shellcheck shell=sh
# Attaches interactive SSH logins to the user's session container.
#
# Installed as /etc/profile.d/fw-session.sh by add-user.sh. Sourced, not
# executed, so it must not exit the shell.
#
# Leaving the session returns to this host shell rather than logging out, which
# is where `fw-session destroy` is run from.

__fw_session_attach() {
    # Interactive logins only. `ssh host command`, scp, rsync and git over ssh
    # use non-login shells and never reach this file, but guard anyway.
    case "$-" in
        *i*) ;;
        *) return 0 ;;
    esac

    # Local console logins are left alone: an administrator fixing a broken
    # machine should not be dropped into a container.
    [ -n "${SSH_CONNECTION:-}" ] || return 0

    command -v fw-session > /dev/null 2>&1 || return 0

    # Accounts without a data directory are not session users.
    [ -d "/mnt/nvme/$(id -un)" ] || return 0

    # Administrators get a host shell: every runbook operates on the host, and
    # running one inside a session would configure the container instead. They
    # start a session with `fw-session` when they want one.
    if id -nG 2>/dev/null | tr ' ' '\n' | grep -qx sudo; then
        return 0
    fi

    fw-session || echo "Session unavailable; you are on the host." >&2

    echo ""
    echo "You are now on the host. 'fw-session' returns to your session;"
    echo "'fw-session destroy' removes it. Your files are unaffected either way."
}

__fw_session_attach
unset -f __fw_session_attach
