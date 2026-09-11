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

    # Accounts without a data directory are not session users. This is how
    # shared administrative accounts keep a plain host shell.
    [ -d "/mnt/nvme/$(id -un)" ] || return 0

    fw-session || echo "Session unavailable; you are on the host." >&2

    echo ""
    echo "You are now on the host. 'fw-session' returns to your session;"
    echo "'fw-session destroy' removes it. Your files are unaffected either way."
}

__fw_session_attach
unset -f __fw_session_attach
