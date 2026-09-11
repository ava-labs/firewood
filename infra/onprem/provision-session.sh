#!/bin/bash
# Provisions a session container image. Runs as root inside a freshly
# launched Ubuntu 26.04 system container; the result is baked into an image
# with `lxc publish`. See ADMINISTRATION.md.
#
# The toolchains mirror .devcontainer/features/firewood-tools/install.sh so
# that a session and a devcontainer offer the same tools. Neither pins
# versions: `apt`, `rustup` and `cargo binstall` all fetch what is current at
# build time. That is why the built image, not this script, is the artifact of
# record.
#
# The image carries no session account. fw-session creates one per instance
# matching the host user's name, uid and gid, so a single image serves
# everyone and `~` inside the session is the same path as on the host, where
# their home directory is mounted from. Baking in a fixed account would mean
# depending on a particular uid being free, and would leave `~` pointing
# somewhere other than the mounted home.
set -o errexit
set -o nounset
set -o pipefail

# Most Ubuntu mirrors measured in the hundreds of bytes per second from these
# machines while the host link ran at ~87 MB/s. Azure's was the exception, so
# it is the default. That is a starting point rather than a verdict: the
# likeliest explanation is that resolute was newly released and the other
# mirrors were still syncing or overloaded, in which case this default stops
# being the right one. The script measures whatever it is told to use.
APT_MIRROR=azure.archive.ubuntu.com
# Below this, the script warns that the mirror is the problem rather than
# letting a multi-hour stall look like a broken machine.
SLOW_MIRROR_THRESHOLD=1000000

show_usage() {
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Provisions a session container image. Run inside the container."
    echo ""
    echo "Options:"
    echo "  --apt-mirror HOST        Apt mirror to use"
    echo "                           (default: azure.archive.ubuntu.com)"
    echo "  --keep-apt-mirror        Leave the image's own mirror in place"
    echo "  --help                   Show this help message"
}

while [[ $# -gt 0 ]]; do
    case $1 in
        --apt-mirror)
            APT_MIRROR="$2"
            shift 2
            ;;
        --keep-apt-mirror)
            APT_MIRROR=""
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
    echo "This script must be run as root, inside the container" >&2
    exit 1
fi

export RUSTUP_HOME=/usr/local/rustup
export CARGO_HOME=/usr/local/cargo
export GOROOT=/usr/local/go
export GOPATH=/go
export PATH="$CARGO_HOME/bin:$GOROOT/bin:$GOPATH/bin:$PATH"
export DEBIAN_FRONTEND=noninteractive

GO_VERSION=1.26.0

step() {
    echo ""
    echo "=== $* ==="
}

step "Base packages"

if [ -n "$APT_MIRROR" ]; then
    # Ubuntu 24.04 and later keep sources in deb822 format under
    # /etc/apt/sources.list.d/, with the older sources.list as a fallback.
    echo "Switching apt mirror to $APT_MIRROR"
    sed -i -E "s|https?://[a-z0-9.-]*archive\.ubuntu\.com|http://${APT_MIRROR}|g" \
        /etc/apt/sources.list.d/ubuntu.sources /etc/apt/sources.list 2>/dev/null || true
    grep -hoE 'https?://[a-z0-9./-]+' /etc/apt/sources.list.d/ubuntu.sources 2>/dev/null |
        sort -u | head -3

    # A slow mirror is the difference between minutes and hours here, and it
    # presents as the machine being broken. Say so up front.
    # shellcheck source=/dev/null
    codename="$(. /etc/os-release && echo "$VERSION_CODENAME")"
    speed="$(curl -o /dev/null -w '%{speed_download}' -s --max-time 30 \
        "http://${APT_MIRROR}/ubuntu/dists/${codename}/main/binary-amd64/Packages.gz" \
        2>/dev/null || echo 0)"
    printf 'Mirror throughput: %.0f B/s\n' "$speed"

    if awk -v s="$speed" -v t="$SLOW_MIRROR_THRESHOLD" 'BEGIN { exit !(s < t) }'; then
        cat <<HINT

Warning: $APT_MIRROR is measuring slow. Provisioning will continue, but
expect hours rather than minutes.

To use a different mirror, interrupt this, compare candidates from the host,
and rerun with --apt-mirror:

  for m in archive.ubuntu.com azure.archive.ubuntu.com mirrors.kernel.org; do
    printf '%-28s ' "\$m"
    curl -o /dev/null -s --max-time 20 -w '%{speed_download} B/s\n' \\
      "http://\$m/ubuntu/dists/${codename}/main/binary-amd64/Packages.gz"
  done

HINT
    fi
fi

# Several of the packages below live in universe. The Ubuntu container image
# enables it already, but another base image might not; note that a mirror can
# also list universe in Components while carrying no index for it, which
# presents as those packages simply not existing.
for component in universe multiverse; do
    if ! grep -qE "^Components:.*\b${component}\b" \
        /etc/apt/sources.list.d/ubuntu.sources 2>/dev/null; then
        echo "Enabling $component"
        sed -i -E "s/^(Components:.*)$/\1 ${component}/" \
            /etc/apt/sources.list.d/ubuntu.sources
    fi
done

apt-get update

# pkg-config is transitional in 24.04 and later; pkgconf is the real package.
PACKAGES=(
    build-essential
    ca-certificates
    clang
    cmake
    curl
    git
    jq
    libssl-dev
    pkgconf
    protobuf-compiler
    shellcheck
    sudo
    tmux
    xz-utils
)

# Let apt resolve the whole list first, so an unavailable package is reported
# by name, with apt's own diagnosis, before anything is installed. Package
# names move between Ubuntu releases and this will happen again.
if ! dry_run_output="$(apt-get install --dry-run -qq "${PACKAGES[@]}" 2>&1)"; then
    echo "" >&2
    echo "Error: apt cannot install the requested package list." >&2
    echo "" >&2
    echo "$dry_run_output" >&2
    echo "" >&2
    echo "Enabled components:" >&2
    grep -h '^Components:' /etc/apt/sources.list.d/ubuntu.sources >&2 || true
    echo "" >&2
    echo "Check names with 'apt-cache search' inside the container." >&2
    exit 1
fi

apt-get install -y --no-install-recommends "${PACKAGES[@]}"
rm -rf /var/lib/apt/lists/*

step "Rust"

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs |
    sh -s -- -y --no-modify-path --profile default
rustup component add llvm-tools rust-docs
rustup toolchain install nightly --profile minimal
rustup component add clippy rustfmt rust-src rust-docs llvm-tools --toolchain nightly

step "Go"

curl -fsSL "https://go.dev/dl/go${GO_VERSION}.linux-amd64.tar.gz" |
    tar -C /usr/local -xz
mkdir -p "$GOPATH"

step "Cargo tools"

curl -L --proto '=https' --tlsv1.2 -sSf \
    https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash

cargo binstall --no-confirm --locked \
    ast-grep \
    cargo-edit \
    cargo-expand \
    cargo-machete \
    cargo-msrv \
    cargo-nextest \
    git-cliff \
    just \
    ripgrep \
    rustfilt \
    sccache

step "Go tools"

go install github.com/reteps/dockerfmt@latest
go install mvdan.cc/sh/v3/cmd/shfmt@latest
go clean -cache -testcache -modcache -fuzzcache

step "Shell environment"

# System-wide, so it applies however the session is entered.
#
# Source code lives in the home directory, which comes from the host's SATA
# system disk. Build artefacts and caches are hot and large, so they are
# redirected to the NVMe array instead. Both survive the instance, which is
# discarded.
#
# The baked toolchains stay root-owned and read-only. Anything a user installs
# goes to their own directory instead, so no part of this image needs to be
# writable by a session account that does not exist yet at build time.
cat > /etc/profile.d/firewood-session.sh <<'PROFILE'
export RUSTUP_HOME=/usr/local/rustup
export CARGO_HOME=/usr/local/cargo
export GOROOT=/usr/local/go
export PATH="$CARGO_HOME/bin:$GOROOT/bin:/go/bin:$PATH"

# ~/firewood points at this user's directory on the NVMe array.
if [ -d "$HOME/firewood" ]; then
    export CARGO_TARGET_DIR="$HOME/firewood/target"
    export CARGO_INSTALL_ROOT="$HOME/firewood/cargo"
    export SCCACHE_DIR="$HOME/firewood/.sccache"
    export RUSTC_WRAPPER="$CARGO_HOME/bin/sccache"
    export GOPATH="$HOME/firewood/go"
    export GOCACHE="$HOME/firewood/.gocache"
    export PATH="$CARGO_INSTALL_ROOT/bin:$GOPATH/bin:$PATH"
fi
PROFILE
chmod 0644 /etc/profile.d/firewood-session.sh

step "Cleanup"

# Drop build-time caches so the published image stays small. CARGO_HOME stays
# root-owned and read-only; users build into CARGO_TARGET_DIR and install into
# CARGO_INSTALL_ROOT, both under their own data directory.
rm -rf "$CARGO_HOME/registry" "$CARGO_HOME/git" /root/.cache /go/pkg

step "Verification"

# shellcheck source=/dev/null
. /etc/profile.d/firewood-session.sh
rustup show
go version
sccache --version
just --version
echo ""
echo "Provisioned. Stop the instance and publish it."
