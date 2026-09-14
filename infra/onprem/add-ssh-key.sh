#!/bin/bash
# Installs a user's SSH public key, so they can log in before Cloudflare's
# certificate authority is in place.
#
# INTERIM. Once the CA is configured, Cloudflare mints a short-lived
# certificate per connection and no keys need to exist on these machines.
# Remove them then: see SETUP.md.
#
# Takes the key on stdin or as a file, and refuses anything that is not a
# public key, since a private key pasted here by accident would be a
# disclosure rather than a typo.
set -o errexit
set -o nounset
set -o pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
USER_MANIFEST="$SCRIPT_DIR/../users/firewood-users.yaml"

DRY_RUN=0
USERNAME=""
KEY_FILE=""
LAUNCH_USER=""

show_usage() {
	echo "Usage: $0 [OPTIONS] USERNAME [KEYFILE]"
	echo ""
	echo "Appends a public key to USERNAME's authorized_keys."
	echo "Reads stdin when KEYFILE is omitted."
	echo ""
	echo "Options:"
	echo "  --launch-user NAME       Take the key from NAME's entry in"
	echo "                           infra/users/firewood-users.yaml"
	echo "  --dry-run                Print what would be done, change nothing"
	echo "  --help                   Show this help message"
	echo ""
	echo "The benchmark hosts and these machines name the same people"
	echo "differently, so --launch-user is the name in the YAML and USERNAME is"
	echo "the local account: --launch-user rkuris ron.kuris"
}

while [[ $# -gt 0 ]]; do
	case $1 in
	--launch-user)
		LAUNCH_USER="$2"
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
		if [ -z "$USERNAME" ]; then
			USERNAME="$1"
		elif [ -z "$KEY_FILE" ]; then
			KEY_FILE="$1"
		else
			echo "Error: too many arguments" >&2
			exit 1
		fi
		shift
		;;
	esac
done

if [ -z "$USERNAME" ]; then
	echo "Error: no username given" >&2
	show_usage
	exit 1
fi

if [ "$EUID" -ne 0 ]; then
	echo "This script must be run as root" >&2
	exit 1
fi

if ! id -u "$USERNAME" >/dev/null 2>&1; then
	echo "Error: no such user '$USERNAME'" >&2
	exit 1
fi

if [ -n "$LAUNCH_USER" ] && [ -n "$KEY_FILE" ]; then
	echo "Error: --launch-user and KEYFILE both name a key" >&2
	exit 1
fi

if [ -n "$LAUNCH_USER" ]; then
	if [ ! -f "$USER_MANIFEST" ]; then
		echo "Error: $USER_MANIFEST not found" >&2
		exit 1
	fi
	# First ssh_authorized_keys entry under the matching benchmark_user.
	KEY="$(awk -v want="$LAUNCH_USER" '
        function trim(s) {
            sub(/^[[:space:]]+/, "", s)
            sub(/[[:space:]]+$/, "", s)
            return s
        }
        function value(line) {
            sub(/^[^:]*:[[:space:]]*/, "", line)
            line = trim(line)
            gsub(/^"|"$/, "", line)
            return line
        }
        function list_value(line) {
            sub(/^[[:space:]]*-[[:space:]]*/, "", line)
            line = trim(line)
            gsub(/^"|"$/, "", line)
            return line
        }
        /^  - local_user:/ {
            current = ""
            collecting = 0
            next
        }
        /^    benchmark_user:/ {
            current = value($0)
            next
        }
        current == want && /ssh_authorized_keys:/ { collecting = 1; next }
        collecting {
            if ($1 != "-") exit
            print list_value($0)
            exit
        }
    ' "$USER_MANIFEST")"
	if [ -z "$KEY" ]; then
		echo "Error: no key for '$LAUNCH_USER' in $USER_MANIFEST" >&2
		echo "Known names:" >&2
		awk '
            function trim(s) {
                sub(/^[[:space:]]+/, "", s)
                sub(/[[:space:]]+$/, "", s)
                return s
            }
            /^    benchmark_user:/ {
                sub(/^[^:]*:[[:space:]]*/, "")
                print "  " trim($0)
            }
        ' "$USER_MANIFEST" >&2
		exit 1
	fi
	echo "Using $LAUNCH_USER's key from $(basename "$USER_MANIFEST")"
elif [ -n "$KEY_FILE" ]; then
	KEY="$(cat "$KEY_FILE")"
else
	echo "Paste the public key, then ctrl-d:" >&2
	KEY="$(cat)"
fi

KEY="$(echo "$KEY" | tr -d '\r' | sed '/^[[:space:]]*$/d')"

if [ "$(echo "$KEY" | wc -l)" -ne 1 ]; then
	echo "Error: expected exactly one key, got $(echo "$KEY" | wc -l) lines" >&2
	exit 1
fi

# A private key here would be a disclosure, not a typo. Refuse loudly.
case "$KEY" in
*PRIVATE\ KEY*)
	echo "Error: that is a PRIVATE key. Do not share it with anyone," >&2
	echo "including this machine. Ask for the .pub file instead." >&2
	exit 1
	;;
ssh-ed25519\ * | ssh-rsa\ * | ecdsa-sha2-* | sk-ssh-ed25519\ * | sk-ecdsa-sha2-*)
	;;
*)
	echo "Error: does not look like an SSH public key." >&2
	echo "Expected a line beginning ssh-ed25519, ssh-rsa or ecdsa-sha2-." >&2
	exit 1
	;;
esac

if command -v ssh-keygen >/dev/null 2>&1; then
	if ! echo "$KEY" | ssh-keygen -l -f /dev/stdin >/dev/null 2>&1; then
		echo "Error: ssh-keygen does not recognise that key" >&2
		exit 1
	fi
	echo "Key: $(echo "$KEY" | ssh-keygen -l -f /dev/stdin)"
fi

USER_HOME="$(getent passwd "$USERNAME" | cut -d: -f6)"
AUTH_KEYS="$USER_HOME/.ssh/authorized_keys"

if [ "$DRY_RUN" -eq 1 ]; then
	echo "+ append to $AUTH_KEYS"
	exit 0
fi

install -d -m 0700 -o "$USERNAME" -g "$USERNAME" "$USER_HOME/.ssh"
touch "$AUTH_KEYS"

if grep -qxF "$KEY" "$AUTH_KEYS" 2>/dev/null; then
	echo "Already present in $AUTH_KEYS"
else
	echo "$KEY" >>"$AUTH_KEYS"
	echo "Appended to $AUTH_KEYS"
fi

# sshd ignores authorized_keys that anyone but the owner can write.
chmod 0600 "$AUTH_KEYS"
chown "$USERNAME:$USERNAME" "$AUTH_KEYS"

echo ""
echo "'$USERNAME' can now log in with that key, through the tunnel as usual."
echo "Interim only: remove these keys once the Cloudflare CA is in place."
