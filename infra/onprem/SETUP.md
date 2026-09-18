# Setting Up the On-Prem Servers

How `snoopy` and `linus` are built and maintained. For day-to-day use, see
[README.md](README.md).

This document has two parts. [Design](#design) explains the choices.
[Runbooks](#runbooks) gives the command sequences.

## Design

These on-prem machines are configured with these goals in mind:

- Predictable benchmark performance. Minimize sharing and virtual layers
  between the code and the hardware.
- Cheap environment reset. Change things freely, then return to a known state
  with one command.
- Minimal cost of administration and networking
- Maximum security -- zero trust access.

### Storage

[`setup-nvme.sh`](setup-nvme.sh) runs once per machine. It stripes every empty
NVMe device into one LVM volume group, formats it ext4, mounts it at
`/mnt/nvme`, and makes it writable by the `firewood` group. It refuses any
device holding a filesystem, partition, mount, or LVM physical volume.

- LVM striping rather than mdadm. LVM is already in use for the root volume,
  device-mapper paths survive reboots where `/dev/md0` can reappear as
  `/dev/md127`, and a single drive can later be carved out of the volume group
  for single-disk comparisons without rebuilding the array.
- The fstab entry uses `UUID=` and `nofail`. Without `nofail` a failed mount
  drops Ubuntu to an emergency console, which costs a trip to the IDF; with it,
  the machine still boots and is reachable over SSH.
- `bytes-per-inode` is 65536, deliberately **not** the 2097152 that
  `benchmark/setup-scripts/build-environment.sh` uses for the EC2 equivalent.
- The script refuses to proceed when any NVMe device is unusable, rather than
  striping across the remainder. Snoopy ran for a while on one of four devices
  because three still held an old mdadm array and the script skipped them with
  only a note. A machine quietly using a quarter of its array produces
  benchmark numbers that mean nothing. `--allow-partial` overrides this when
  the shortfall is intentional.
- `bytes-per-inode` is 65536. The EC2 script uses 2 MB per inode, but that
  gives too few inodes for Rust target directories and container images. The
  saved inode-table space is about 1% of a 7.3 TB volume.

### Where files live

| Path | Device | Holds |
| --- | --- | --- |
| `/` | SATA system disk, 98 GB | the OS |
| `/home` | own volume on the root volume group | source checkouts, dotfiles |
| `/mnt/nvme/<user>` | striped NVMe array | databases, chain state, build artifacts, caches |

Home directories get their own logical volume via
[`setup-home.sh`](setup-home.sh), run once per machine. Otherwise they would
share the 98 GB root filesystem, which a couple of Rust target directories can
fill. The root volume group has terabytes unallocated, so home can be expanded
later.

Source code lives in home because it does not need performance.
`target/`, the cargo registry, the sccache directory and the Go build cache are
hot and large, so the session environment redirects them to `/mnt/nvme/<user>`
through `CARGO_TARGET_DIR`, `CARGO_HOME`, `SCCACHE_DIR` and `GOCACHE`. People
work in `~/...` and the I/O-heavy parts land on the fast disk without anyone
arranging it.

### Accounts

Local accounts are created on each machine and added to the `firewood` group,
which grants write access to `/mnt/nvme`. This is per-machine because the
hosts do not communicate directly, and adding central account infrastructure is
not worth the current complexity.

Accounts are local and not synchronized with Okta. With two machines and a
small team, manual accounts cost less than the extra infrastructure. Revisit
if either number grows.

#### Who has an account

[`../users/firewood-users.yaml`](../users/firewood-users.yaml) is the shared
desired user list for benchmark hosts and on-prem accounts. The machines are
still the operational source of truth:

```bash
getent group firewood
```

- `ron.kuris`
- `juan.leon`
- `amin.rezaei`
- `austin.larson`
- `brandon.leblanc`
- `rodrigo.villar`
- `bernard`
- `felipe.madero`

The same accounts should exist on both machines. Update the shared manifest
when adding or removing users, then run `add-users.sh` on each machine.

### Access plumbing

Access relies on these Cloudflare tunnels:

| Hostname | Forwards to |
| --- | --- |
| `ethchallenge.avax-dev.network` | `snoopy` sshd |
| `ethchallenge2.avax-dev.network` | `linus` sshd |
| `ethchallenge3.avax-dev.network` | PiKVM, `https://localhost` |

The first two tunnels are for regular use. The third is for console tasks such
as firmware or OS updates. Both hosts trust Cloudflare's SSH certificate
authority. Cloudflare mints a short-lived certificate per connection and the
host verifies it against that CA, so no per-user keys exist on the machines.

The `~/.ssh/config` stanza people need in order to reach the machines through
these tunnels is in [README.md](README.md#ssh), with the rest of the
user-facing access instructions.

#### Interim: public keys instead of certificates

The certificate authority is not configured yet. Until it is, accounts use
ordinary SSH public keys in `authorized_keys`.

Users cannot install their own key, having no way in yet, so an administrator
places it. Ask them for the **public** half, on each machine:

```bash
sudo bash infra/onprem/add-ssh-key.sh <username> id_ed25519.pub
```

It refuses anything that is not a public key, and says so loudly if handed a
private one.

Most of the team already has a key recorded in
[`../users/firewood-users.yaml`](../users/firewood-users.yaml), so there is
nothing to ask them for. That file records both benchmark and on-prem account
names, so `--launch-user` still accepts the benchmark name:

```bash
sudo bash infra/onprem/add-ssh-key.sh --launch-user rkuris ron.kuris
```

Run it with an unknown name to list the ones it knows.

When the CA arrives: configure it, confirm a certificate login works, then
remove the keys, since leaving them means two ways in and only one of them
gets revoked when someone leaves.

```bash
sudo rm /home/<username>/.ssh/authorized_keys
```

The tunnels themselves, the DNS records, and the certificate authority are
managed by the security team. A machine rebuilt from scratch needs a request to
them for the tunnel and CA. Everything else here is reproducible from this
repository.

The PiKVM's port numbering is inverted relative to the DNS names: port 1 is
`linus`, port 2 is `snoopy`. Correcting this requires physical access to the
IDF; do it when convenient.

Besides each user name for each member of the team, we have these users:

| User Name | Where | What for |
| --- | --- | --- |
| firewood | snoopy, linus | `sudo` user on hosts |
| root | PiKVM | Management of Linux on switch |
| admin | PiKVM | Management of web UI credentials on switch |

The passwords for these users are in 1Password. Ask someone who knows.

### Package parity

The hardware already differs (CPU bin, NVMe generation). Software should not
add more divergence: keep the package and snap sets the same on both. Watch
for extras pulled in by the Ubuntu installer, which are easy to select by
accident and show up later as unexplained differences.

[PACKAGES.md](PACKAGES.md) records what is installed beyond a vanilla Ubuntu
26.04 server install. Update it whenever something is added, and diff the two
machines against each other after any change.

### Sharing package lists with the benchmark hosts

Not done yet. The goal is that the session image derives its packages and tools
from the benchmark definition and adds to them, instead of keeping a second
copy that drifts.

Rust and Go are already single-sourced.
[`firewood-toolchain.sh`](../toolchains/firewood-toolchain.sh) pins the
toolchains, `FIREWOOD_CARGO_TOOLS` and `FIREWOOD_GO_TOOLS`, and is sourced by
`benchmark/setup-scripts/install-rust.sh`,
`benchmark/setup-scripts/install-golang.sh` and
[`provision-session.sh`](provision-session.sh). What still drifts is apt
packages, and how a few tools are installed.

The benchmark path installs from five places:

| Where | What |
| --- | --- |
| `launch-stages.yaml`, `packages:` | cloud-init apt list |
| `benchmark/setup-scripts/build-environment.sh` | apt, plus `mdadm` and `zfsutils-linux` |
| `benchmark/setup-scripts/install-grafana.sh` | `grafana`, `prometheus` |
| `launch-stages.yaml`, snaps | `amazon-ssm-agent`, `task` |
| `launch-stages.yaml`, `install-s5cmd` | newest s5cmd `.deb`, resolved through the GitHub API |

Divergences as of 2026-09-16:

- s5cmd and `task` are pinned in `firewood-toolchain.sh` for sessions, while the
  launch path takes whatever s5cmd is newest at launch and `task` from a snap.
  Pinning the launch path to those versions is the smallest useful first step.
- The session image carries `clang`, `cmake`, `pkgconf`, `libssl-dev`,
  `shellcheck`, `tmux` and `xz-utils`, and every entry in
  `FIREWOOD_CARGO_TOOLS`. The benchmark hosts build the FFI without them, so
  the shared set has to be settled on evidence rather than by taking the union.
- `mdadm`, `zfsutils-linux`, `amazon-ssm-agent`, `grafana` and `prometheus`
  are EC2 or host concerns. On-prem, [`setup-nvme.sh`](setup-nvme.sh) owns
  disks and the observability stack belongs on the host, not in a container
  discarded by `fw-session recreate`.
- `make` is in the launch list and already in `build-essential`.

So "benchmark list plus additions" needs the shared definition to separate the
common set from the host-only one; a single list that sessions extend pulls
EC2-only packages into the image.

### Session images

Logging in over SSH attaches to a persistent per-user container managed by
`fw-session`. A session is created or joined on login from a shared image, and
removed only when its owner runs `fw-session destroy`.

Sessions persist because a dropped SSH connection must not kill a multi-hour
run. The tradeoff is image drift. `fw-session` reports when a newer image
exists, so benchmark work can start from `fw-session recreate`.

Containers give cheap reset to a known software environment. They are not meant
to make the hardware itself repeatable; page cache, thermal state and NVMe
cache state are host concerns.

- LXD system containers, one per user. `lxc launch --vm` is available for work
  needing its own kernel, at the cost of I/O fidelity. LXD rather than Incus
  because it is installed by the Ubuntu bootstrap workflow.
- LXD's multi-user daemon gives each member of the `firewood` group a confined
  project of their own, created on first use and named `user-<uid>`. Members of
  the `lxd` group would instead get full administrative access, which is
  equivalent to root on the host.
- Confinement is what makes root inside a session acceptable. A confined user
  can attach disk devices only with sources under the prefixes in
  `restricted.devices.disk.paths`, set by `add-user.sh` to their home and data
  directories.
- [`provision-session.sh`](provision-session.sh) builds the session image and
  installs the toolchains from
  [`../toolchains/firewood-toolchain.sh`](../toolchains/firewood-toolchain.sh).
  Rust, Go, cargo-binstall, Cargo tools and Go tools are pinned there so image
  rebuilds and benchmark hosts do not silently change the developer
  environment. `.devcontainer/` cannot be reused directly: it is an OCI image
  assembled from devcontainer features, while a system container boots
  `systemd`.
- The image is shared by all accounts/sessions. `fw-session` configures each
  session to match the user's name, uid and gid, so `~` inside the session is
  the same path as outside on the bare machine, where their home is mounted.
  The baked toolchains stay root-owned and are shared through `PATH` alone;
  `CARGO_HOME`, `CARGO_TARGET_DIR`, `CARGO_INSTALL_ROOT`, `GOPATH` and the
  caches point into the user's data directory, because cargo writes its
  registry and git checkouts into `CARGO_HOME` on the first build.
- Build the image once and copy it to the other machine. The same script run
  twice does not produce the same image, and the hosts cannot reach each other,
  so transfer is `lxc image export`/`import` through an external location.
- `/etc/profile.d/fw-session.sh`, installed by `add-user.sh`, enters the
  session on SSH login. Console logins, accounts with no data directory, and
  members of `sudo` bypass it. Administrators get a host shell because the
  runbooks below operate on the host; running one inside a session would
  configure the container.

### Apt mirrors

The session image build defaults to `azure.archive.ubuntu.com`, not the usual
mirror, only because the standard mirrors were extremely slow when this was
configured. Treat the default as a starting point: `provision-session.sh`
measures whatever it is given and warns under
1 MB/s. Only apt is covered; `rustup`, `go.dev` and GitHub releases are
separate, so a stall in the `Rust`, `Go` or `Cargo tools` step requires a
different adjustment.

To compare candidates from a host:

```bash
for m in archive.ubuntu.com azure.archive.ubuntu.com mirrors.kernel.org; do
  printf '%-28s ' "$m"
  curl -o /dev/null -w '%{speed_download} B/s\n' -s --max-time 20 \
    "http://$m/ubuntu/dists/resolute/main/binary-amd64/Packages.gz" || echo fail
done
```

### Apt sources and cloud-init

`/etc/apt/sources.list.d/ubuntu.sources` is generated by cloud-init, which
rewrites it when its `config-apt` stage runs. Provisioning a container before
that finishes loses both edits `provision-session.sh` makes to it: the mirror,
and the `universe` component.

Losing the mirror is merely slow. Losing `universe` fails the build much later,
on the package names that live there:

```text
E: Unable to locate package clang
E: Unable to locate package cmake
E: Unable to locate package protobuf-compiler
E: Unable to locate package shellcheck
```

That reads as wrong package names rather than a clobbered file, and being a
race, it is intermittent. The script waits for `cloud-init status --wait` and
reads its own edits back, so a clobbered file fails with the sources file in
the output instead of apt reporting names. If a build stops there, wait for
cloud-init and rerun rather than re-editing by hand.

Recent Ubuntu container images enable `universe` already, which makes the edit
a no-op. Confirm before provisioning:

```bash
sudo lxc exec build-tmp -- cloud-init status --wait
sudo lxc exec build-tmp -- grep -n 'URIs:\|Components:' \
    /etc/apt/sources.list.d/ubuntu.sources
```

### Toolchains

The hosts carry Nix 2.34.3 and nothing else. No language toolchains are
installed on them.

Toolchains are in the repeatable session image.

`nix develop` against `ffi/flake.nix` provides pinned toolchains on the host
for work that should not run in a session.

Inside a session the toolchains are shared and root-owned, so `RUSTUP_HOME` is
read-only: building works, `rustup component add` does not. `sudo -E` covers a
one-off, but writes into the shared tree and is lost on `fw-session recreate`.
Anything needed repeatedly belongs in `firewood-toolchain.sh`.

### Known gaps

- `/mnt/nvme` has no redundancy or backup. One NVMe failure loses user data and
  the LXD pool. `/home` is also a single volume on a single SSD.
- There are no quotas. One user filling `/mnt/nvme` can stop every session on
  the machine.
- Benchmark repeatability may need host-level resets such as `drop_caches` and
  `fstrim`.
- `io_uring` in sessions is unverified. Firewood enables `cfg(io_uring)` on
  Linux, so a session could exercise a different I/O path than production.
- Session I/O has not been compared with bare metal. A `fio` run and a short
  re-execution on host and session would settle this.
- C-Chain state has no agreed location. A shared read-only copy under
  `/mnt/nvme` is worth considering.
- Logins use public keys until the Cloudflare CA is configured. Remove
  `authorized_keys` once certificate login works.
- Reservations are advisory. Nothing records or enforces who holds a machine.
- Package lists are duplicated between the benchmark hosts and the session
  image, and drift. See [Sharing package lists with the benchmark
  hosts](#sharing-package-lists-with-the-benchmark-hosts).
- Nothing tracks image size, and the Go tools dominate it: `task` and s5cmd are
  roughly 70 MB and 20 MB. It matters because the image moves between the
  machines by hand. Provisioning already strips the build and module caches, so
  what is left is binaries; the options are `-ldflags=-s -w` or dropping tools
  nobody uses.

## Runbooks

Every `lxc` command here needs `sudo`. Administrators are not in the `lxd`
group — membership there is equivalent to root on the host — so without it
they cannot reach the daemon at all, and once `daemon.user.group` is set a
bare `lxc` operates in their own confined project instead.

Run these on the host. If you are in a session, `exit` first.

Bringing up a machine from scratch, in order. Each step verifies before the
next depends on it:

1. Install OS through the KVM switch (not covered here). Include the packages
   this setup assumes, especially `lxc`. `PACKAGES.md` may help.
2. [Configure the NVMe array](#configure-the-nvme-array)
3. [Move /home onto its own volume](#move-home-onto-its-own-volume)
4. [Initialise LXD](#initialise-lxd) onto the NVMe array
5. [Build a session image](#build-a-session-image-and-push-it-to-both-machines)
6. [Add a user](#add-a-user) for each team member

Steps 1 to 4 are per machine. Step 5 runs on one machine and the image is
copied to the other. Step 6 is per machine, per person.

### Configure the NVMe array

Once per machine. Destroys all data on the NVMe devices.

```bash
sudo bash infra/onprem/setup-nvme.sh --dry-run   # check the device list
sudo bash infra/onprem/setup-nvme.sh
```

If a device is already in use, the script names what is on it and stops.
Inspect before deciding whether it is disposable:

```bash
lsblk -f /dev/nvme0n1 /dev/nvme1n1 /dev/nvme2n1 /dev/nvme3n1
sudo mdadm --detail /dev/md127          # if an array is present
```

Then rerun with `--wipe`, which stops any array, zeroes its RAID superblock,
and clears remaining signatures:

```bash
sudo bash infra/onprem/setup-nvme.sh --wipe
```

`--wipe` refuses any device that is mounted or belongs to a volume group.

`--help` lists the options. Reboot afterwards and confirm the mount returns:

```bash
sudo reboot
df -hT /mnt/nvme && sudo lvs firewood
```

### Move /home onto its own volume

If you installed the OS with `/home` on a separate partition, skip this step.
Otherwise, run it once per machine, with nobody else logged in. The script
refuses to run otherwise, since copying home while someone is writing to it
risks inconsistent state.

Two wrinkles, because this step probably replaces the filesystem holding this
checkout:

1. Run it from outside `/home`.
2. Run a copy from `/tmp`, since bash reads a script as it executes and the
   original is about to be masked by the new mount.

```bash
install -m 0755 infra/onprem/setup-home.sh /tmp/setup-home.sh
cd /
sudo bash /tmp/setup-home.sh --dry-run
sudo bash /tmp/setup-home.sh
sudo reboot
df -hT /home
```

It copies rather than moves: the old contents stay on the root filesystem,
hidden under the new mount, until you reclaim that space deliberately.

### Initialise LXD

Once per machine. LXD ships installed but uninitialised, and without a preseed
its storage pool lands on the SATA root disk.

Confirm the array is mounted first. If it is not, `lxd init` creates
`/mnt/nvme/lxd` as an ordinary directory on the root disk and everything works
while sitting on the wrong device:

```bash
df -hT /mnt/nvme && sudo lvs firewood
```

Then:

The preseed only applies to an uninitialised LXD. Check first, because it will
not rename an existing pool and the failure is quiet:

```bash
sudo lxc storage list     # expect no pools at all
```

```bash
sudo mkdir -p /mnt/nvme/lxd
sudo lxd init --preseed < infra/onprem/lxd-init.yaml
sudo lxc storage list     # expect pool 'default', source /mnt/nvme/lxd
```

If a pool already exists under another name, it has to be replaced: the
multi-user daemon writes `pool: default` into every project it creates, so any
other name breaks every confined user. Nothing here is precious at this stage,
but check `sudo lxc list --all-projects` first:

```bash
sudo lxc image list --format csv -c f | xargs -r -n1 sudo lxc image delete
sudo lxc profile device remove default root
sudo lxc storage delete <old-name>
sudo lxc storage create default dir source=/mnt/nvme/lxd
sudo lxc profile device add default root disk path=/ pool=default
```

Then hand the `firewood` group confined access. This is a snap option, not LXD
configuration, so the preseed cannot set it, and without it no confined
projects exist and nobody can run `lxc` at all:

```bash
sudo snap set lxd daemon.user.group=firewood
sudo snap get lxd daemon.user.group      # expect: firewood
```

Do this before adding users. Nobody should be in the `lxd` group, which grants
full administrative access to LXD and is equivalent to root on the host.

### Build a session image and push it to both machines

Runs on one machine; the image is copied to the other.

Every command here uses `sudo`. Once `daemon.user.group` is set, a bare `lxc`
from an administrator's account operates in their own confined project, which
has `features.images false` and cannot hold images. The image has to live in
the default project, which is what confined projects share from.

Build on one machine (for example, `snoopy`):

```bash
TAG=firewood-session-$(date +%Y%m%d)
sudo lxc launch ubuntu:26.04 build-tmp
sudo lxc exec build-tmp -- cloud-init status --wait
sudo lxc exec build-tmp -- mkdir -p /root/infra/onprem /root/infra/toolchains
sudo lxc file push infra/onprem/provision-session.sh build-tmp/root/infra/onprem/
sudo lxc file push infra/toolchains/firewood-toolchain.sh build-tmp/root/infra/toolchains/
sudo lxc exec build-tmp -- bash /root/infra/onprem/provision-session.sh
```

The `cloud-init status --wait` is not optional: see
[Apt sources and cloud-init](#apt-sources-and-cloud-init). The script waits too,
so this is belt and braces on a fresh container.

It reports mirror throughput first. If that warns, or the run drags, pass a
different mirror with `--apt-mirror` and see
[Apt mirrors](#apt-mirrors); `--keep-apt-mirror` uses the image's own sources.

Provisioning ends by printing `rustup show`, `go version`, `sccache --version`
and `just --version`. Check those before publishing: a half-provisioned image
is worse than none.

```bash
sudo lxc stop build-tmp
sudo lxc publish build-tmp --alias "$TAG"
sudo lxc delete build-tmp
sudo lxc image alias delete firewood-session || true
sudo lxc image alias create firewood-session \
    "$(sudo lxc image info "$TAG" | awk '/Fingerprint/ {print $2}')"
sudo lxc image export "$TAG" ~/"$TAG"    # not /tmp: cleared on reboot
echo "$TAG"           # note this; the other machine needs it
```

Then carry the tarball to the other machines (for example, `linus`):

```bash
# on your workstation
scp 'snoopy:firewood-session-*.tar.gz' .
scp firewood-session-*.tar.gz linus:

# on linus: the tag is the tarball's name, so nothing has to be carried over
TARBALL="$(ls -t ~/firewood-session-*.tar.gz | head -1)"
TAG="$(basename "$TARBALL" .tar.gz)"
sudo lxc image import "$TARBALL" --alias "$TAG"
sudo lxc image alias delete firewood-session || true
sudo lxc image alias create firewood-session \
    "$(sudo lxc image info "$TAG" | awk '/Fingerprint/ {print $2}')"
sudo lxc image list
```

Keep the previous image for rollback.

If you lose the tag, it is recoverable: `sudo lxc image alias list` on the
machine that built it shows every alias, including the dated one. Use that
rather than `lxc image list`, whose ALIAS column truncates to "(1 more)". The
exported tarball is also named after the tag, and is the only copy that
survives an LXD reinstall.

#### Updating pinned session tools

All shared tool versions for the session image and benchmark setup live in one
place: [`../toolchains/firewood-toolchain.sh`](../toolchains/firewood-toolchain.sh).
Find the values with:

```bash
rg -n "FIREWOOD_(RUST|GO|CARGO)" infra/toolchains/firewood-toolchain.sh
```

The image build runbook pushes both the provisioner and the manifest into the
temporary container. If another script starts consuming these pins, keep that
script's copy/source step in the same change as the manifest update.

When updating pins:

1. Choose exact versions, not moving channels: `FIREWOOD_RUST_VERSION`,
   dated `FIREWOOD_RUST_NIGHTLY`, `FIREWOOD_GO_VERSION`,
   `FIREWOOD_CARGO_BINSTALL_VERSION`, each `crate@version` in
   `FIREWOOD_CARGO_TOOLS`, and each `module@version` in `FIREWOOD_GO_TOOLS`.
2. Update checksums for the downloaded binaries:
   - `FIREWOOD_RUSTUP_X86_64_LINUX_GNU_SHA256` comes from the
     `rustup-init.sha256` file under:
     `https://static.rust-lang.org/rustup/archive/<version>/x86_64-unknown-linux-gnu/`
   - `FIREWOOD_GO_LINUX_AMD64_SHA256` comes from <https://go.dev/dl/>.
   - `FIREWOOD_CARGO_BINSTALL_X86_64_LINUX_MUSL_SHA256` comes from
     the `cargo-binstall-x86_64-unknown-linux-musl.tgz` asset digest
     in the matching GitHub release.
3. Build a disposable image with the normal commands above and read the final
   verification output.
4. Before publishing, smoke-test the tools that matter for Firewood:

```bash
sudo lxc exec build-tmp -- bash -lc '
  . /etc/profile.d/firewood-session.sh
  rustup show
  go version
  cargo nextest --version
  sccache --version
  just --version
  shfmt --version
  dockerfmt --version
  s5cmd version
  task --version
'
```

`s5cmd version` reports `v0.0.0-dev`: `go install` does not stamp the release
version. The pin in `firewood-toolchain.sh` is what fixes it.

If any install fails because a pinned tool now requires a newer Rust or Go
version, update the language toolchain first and rerun the build from a fresh
`build-tmp`.

### Add a user

[`add-user.sh`](add-user.sh) does everything a new account needs. Run it on
each machine; accounts are per host.

```bash
sudo bash infra/onprem/add-user.sh --full-name "Real Name" <username>
```

It creates the account, joins it to the `firewood` group, creates
`/mnt/nvme/<username>/firewood`, and links it as `~/firewood`. Group
membership takes effect at their next login. `--sudo` adds them to the sudo
group; `--dry-run` shows the steps without running them.

It also installs `fw-session` to `/usr/local/bin`, refreshing it whenever the
repository copy differs, so that is not a step anyone has to remember. Users
start sessions with `fw-session` and need no checkout of this repository.

It then sets up their confined LXD project. The project is created by LXD's
multi-user daemon the first time that user runs any `lxc` command, so the
script triggers that itself rather than waiting for their first login, and
then sets three things on it:

| Setting | Why |
| --- | --- |
| `features.images false` | use the default project's session image |
| `restricted true` | confirm LXD is enforcing the `restricted.*` settings |
| `restricted.devices.disk allow` | permit disk devices at all |
| `restricted.devices.disk.paths` | confine those sources to the user's own directory |

The project must already be restricted: LXD ignores `restricted.*` keys when
`restricted` is false. The disk path list is not optional. `allow` with an
empty paths list permits any host path, which is host root by another route.

Before it finishes, the script validates the confinement boundary it depends
on: the user must not be in the `lxd` group, the LXD snap's
`daemon.user.group` must be the shared team group, the confined project must
have the expected restrictions and disk path allow-list, low-level container
configuration must not be allowed, and the user must see only their own LXD
project.

Password login stays disabled, since Cloudflare authenticates before the
connection reaches the machine. An account that needs `sudo` therefore also
needs a password or a `NOPASSWD` rule, and the script says so when `--sudo` is
used.

When a new per-user setup step appears, add it to `add-user.sh` so one script
stays the complete answer.

To reconcile every account in the shared manifest on a machine:

```bash
sudo bash infra/onprem/add-users.sh --dry-run
sudo bash infra/onprem/add-users.sh
```

This calls `add-user.sh` once per `local_user` in
[`../users/firewood-users.yaml`](../users/firewood-users.yaml), then installs
each listed interim SSH public key. Use `--no-ssh-keys` once Cloudflare
certificate authentication is the only access path.

### Remove a user

[`remove-user.sh`](remove-user.sh) removes the account, their session, their
LXD project and their per-user bridge. Run it on each machine.

```bash
sudo bash infra/onprem/remove-user.sh --dry-run <username>
sudo bash infra/onprem/remove-user.sh <username>
```

Their files are kept by default: the home directory and
`/mnt/nvme/<username>` are left in place, owned by a uid with no account.
Someone leaving often has work others still need, and deleting terabytes of it
is not reversible. `--purge` removes those too, and the default path prints
what to run once you know what is in them.

`--purge` refuses to delete the data directory unless the configured NVMe path
is mounted and the data directory resolves under it.

### Record installed packages

On each machine, then paste each host's output into its section of
[PACKAGES.md](PACKAGES.md):

```bash
bash infra/onprem/inventory-packages.sh
```

Diff the two to confirm the machines still agree.
