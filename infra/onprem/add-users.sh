#!/bin/bash
# Creates every on-prem account listed in infra/users/firewood-users.yaml.
#
# This is a thin coordinator around add-user.sh so the per-user setup sequence
# stays in one place. SSH public keys are installed only as interim access
# until Cloudflare certificate authentication replaces authorized_keys.
set -o errexit
set -o nounset
set -o pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
USER_MANIFEST="$SCRIPT_DIR/../users/firewood-users.yaml"

GROUP_NAME=firewood
MOUNT_POINT=/mnt/nvme
DRY_RUN=0
INSTALL_KEYS=1

show_usage() {
	echo "Usage: $0 [OPTIONS]"
	echo ""
	echo "Creates every on-prem account from infra/users/firewood-users.yaml."
	echo ""
	echo "Options:"
	echo "  --group NAME             Shared group to join (default: firewood)"
	echo "  --mount PATH             NVMe mount point (default: /mnt/nvme)"
	echo "  --no-ssh-keys            Do not install interim authorized_keys"
	echo "  --dry-run                Print what would be done, change nothing"
	echo "  --help                   Show this help message"
}

while [[ $# -gt 0 ]]; do
	case $1 in
	--group)
		GROUP_NAME="$2"
		shift 2
		;;
	--mount)
		MOUNT_POINT="$2"
		shift 2
		;;
	--no-ssh-keys)
		INSTALL_KEYS=0
		shift
		;;
	--dry-run)
		DRY_RUN=1
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

if [ "$EUID" -ne 0 ]; then
	echo "This script must be run as root" >&2
	exit 1
fi

if [ ! -f "$USER_MANIFEST" ]; then
	echo "Error: $USER_MANIFEST not found" >&2
	exit 1
fi

parse_users() {
	awk '
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
	function emit() {
		if (local_user != "") {
			print local_user "\t" full_name "\t" onprem_sudo
		}
	}
	/^  - local_user:/ {
		emit()
		local_user = value($0)
		full_name = ""
		onprem_sudo = "false"
		next
	}
	/^    full_name:/ {
		full_name = value($0)
		next
	}
	/^    onprem_sudo:/ {
		onprem_sudo = value($0)
		next
	}
	END { emit() }
	' "$USER_MANIFEST"
}

parse_keys() {
	awk '
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
		local_user = value($0)
		in_keys = 0
		next
	}
	/^    ssh_authorized_keys:/ {
		in_keys = 1
		next
	}
	/^    [a-z_]+:/ {
		in_keys = 0
		next
	}
	in_keys && /^      - / {
		print local_user "\t" list_value($0)
		next
	}
	' "$USER_MANIFEST"
}

while IFS=$'\t' read -r local_user full_name onprem_sudo; do
	[ -n "$local_user" ] || continue

	args=(
		--group "$GROUP_NAME"
		--mount "$MOUNT_POINT"
	)
	if [ -n "$full_name" ]; then
		args+=(--full-name "$full_name")
	fi
	if [ "$onprem_sudo" = true ]; then
		args+=(--sudo)
	fi
	if [ "$DRY_RUN" -eq 1 ]; then
		args+=(--dry-run)
	fi

	bash "$SCRIPT_DIR/add-user.sh" "${args[@]}" "$local_user"
done < <(parse_users)

if [ "$INSTALL_KEYS" -eq 0 ]; then
	exit 0
fi

while IFS=$'\t' read -r local_user key; do
	[ -n "$local_user" ] || continue
	[ -n "$key" ] || continue

	if [ "$DRY_RUN" -eq 1 ]; then
		echo "+ install interim SSH key for $local_user"
	else
		printf '%s\n' "$key" | bash "$SCRIPT_DIR/add-ssh-key.sh" "$local_user"
	fi
done < <(parse_keys)
