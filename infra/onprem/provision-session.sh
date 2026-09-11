#!/bin/bash
# Provisions a session container image. Runs as root inside a freshly
# launched Ubuntu 26.04 system container; the result is baked into an image
# with `incus publish`. See ADMINISTRATION.md.
#
# The toolchains mirror .devcontainer/features/firewood-tools/install.sh so
# that a session and a devcontainer offer the same tools. Neither pins
# versions: `apt`, `rustup` and `cargo binstall` all fetch what is current at
# build time. That is why the built image, not this script, is the artifact of
# record.
#
# The image is user-agnostic. It carries one `dev` account at uid 1000, and
# the host maps the session owner's uid onto it, so a single image serves
# everyone.
set -o errexit
set -o nounset
set -o pipefail

# Most Ubuntu mirrors have measured in the hundreds of bytes per second from
# these machines, while the host link runs at ~87 MB/s. Azure's has been the
# reliable one, so it is the default; override when it stops being.
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

DEV_USER=dev
DEV_UID=1000
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

Warning: $APT_MIRROR is slow. Compare candidates from the host, then rerun
with --apt-mirror:

  for m in archive.ubuntu.com azure.archive.ubuntu.com mirrors.kernel.org; do
    printf '%-28s ' "\$m"
    curl -o /dev/null -s --max-time 20 -w '%{speed_download} B/s\n' \\
      "http://\$m/ubuntu/dists/${codename}/main/binary-amd64/Packages.gz"
  done

HINT
    fi
fi

apt-get update
apt-get install -y --no-install-recommends \
    build-essential \
    ca-certificates \
    clang \
    cmake \
    curl \
    git \
    jq \
    libssl-dev \
    pkg-config \
    protobuf-compiler \
    shellcheck \
    sudo \
    xz-utils
rm -rf /var/lib/apt/lists/*

step "Session user"

# uid 1000 is what the host idmap targets. Passwordless sudo: the instance is
# ephemeral and rebuilt from this image, so there is nothing to protect.
if ! id -u "$DEV_USER" > /dev/null 2>&1; then
    useradd --create-home --shell /bin/bash --uid "$DEV_UID" "$DEV_USER"
fi
echo "$DEV_USER ALL=(ALL) NOPASSWD:ALL" > "/etc/sudoers.d/$DEV_USER"
chmod 0440 "/etc/sudoers.d/$DEV_USER"

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
# system disk. Build artefacts and the compiler cache are hot and large, so
# they are redirected to the NVMe array instead. Both survive the instance,
# which is discarded.
cat > /etc/profile.d/firewood-session.sh <<'PROFILE'
export RUSTUP_HOME=/usr/local/rustup
export CARGO_HOME=/usr/local/cargo
export GOROOT=/usr/local/go
export GOPATH=/go
export PATH="$CARGO_HOME/bin:$GOROOT/bin:$GOPATH/bin:$PATH"

# ~/firewood points at this user's directory on the NVMe array.
if [ -d "$HOME/firewood" ]; then
    export CARGO_TARGET_DIR="$HOME/firewood/target"
    export SCCACHE_DIR="$HOME/firewood/.sccache"
    export RUSTC_WRAPPER="$CARGO_HOME/bin/sccache"
    export GOCACHE="$HOME/firewood/.gocache"
fi
PROFILE
chmod 0644 /etc/profile.d/firewood-session.sh

# Writable by the session user: cargo installs into CARGO_HOME at runtime.
chown -R "$DEV_USER:$DEV_USER" "$CARGO_HOME" "$GOPATH"

step "Cleanup"

# Drop build-time caches so the published image stays small.
rm -rf "$CARGO_HOME/registry" "$CARGO_HOME/git" /root/.cache
mkdir -p "$CARGO_HOME/registry" "$CARGO_HOME/git"
chown -R "$DEV_USER:$DEV_USER" "$CARGO_HOME"

step "Verification"

# shellcheck source=/dev/null
. /etc/profile.d/firewood-session.sh
rustup show
go version
sccache --version
just --version
echo ""
echo "Provisioned. Stop the instance and publish it."
