# Administering the On-Prem Servers

How `snoopy` and `linus` are built and maintained. For day-to-day use, see
[README.md](README.md).

This document is in two halves. [Design](#design) explains how things are set
up and why, for anyone who needs to understand what they are dealing with.
[Runbooks](#runbooks) is the command sequences on their own, for when you
already understand and just need the steps.

## Design

### Storage

[`setup-nvme.sh`](setup-nvme.sh) stripes every empty NVMe device into one LVM
volume group, formats it ext4, mounts it at `/mnt/nvme`, and makes it writable
by the `firewood` group. It refuses any device holding a filesystem,
partitions, a mount, or an existing LVM physical volume, and is idempotent.

- LVM striping rather than mdadm. LVM is already in use for the root volume,
  device-mapper paths survive reboots where `/dev/md0` can reappear as
  `/dev/md127`, and a single drive can later be carved out of the volume group
  for single-disk comparisons without rebuilding the array.
- The fstab entry uses `UUID=` and `nofail`. Without `nofail` a failed mount
  drops Ubuntu to an emergency console, which costs a trip to the IDF; with it,
  the machine still boots and is reachable over SSH.
- `bytes-per-inode` is 65536, deliberately **not** the 2097152 that
  `benchmark/setup-scripts/build-environment.sh` uses for the EC2 equivalent.
  See [Inode ratio](#inode-ratio).
- The script refuses to proceed when any NVMe device is unusable, rather than
  striping across the remainder. Snoopy ran for a while on one of four devices
  because three still held an old mdadm array and the script skipped them with
  only a note. A machine quietly using a quarter of its array produces
  benchmark numbers that mean nothing. `--allow-partial` overrides this when
  the shortfall is intentional.

#### Inode ratio

`bytes-per-inode` fixes the inode count for the life of the filesystem, so
this is worth getting right once.

The EC2 script uses 2 MB per inode. The reasoning is sound as far as it goes:
each inode costs 256 bytes whether used or not, so on a 7.3 TB volume the
ext4 default of 16 KB per inode preallocates around 460 M inodes costing about
117 GB, while 2 MB per inode yields 3.6 M inodes costing under 1 GB. Fewer
inodes also leave more contiguous space per block group, which marginally
suits Firewood storing its trie in one very large file.

**Assumption, recorded as such:** that figure is treated here as over-tuning
for EC2 that was never revisited, rather than a measured optimum. Nothing
found so far demonstrates the space saving or the locality effect mattering to
Firewood.

Against it: these filesystems also hold container images and Rust target
directories, each running to hundreds of thousands of small files. A 931 GB
volume formatted at 2 MB per inode exhausted its 477 k inodes while unpacking
a single session image, at 60% of its capacity in bytes.

The decision is 65536, which gives roughly 114 M inodes on 7.3 TB for about
29 GB, or 0.4% of the volume. The choice is not 2 MB against 16 KB: at 64 KB
the space argument costs around 1% and the locality effect is well under a
percent, against a filesystem that can actually hold what is put on it.

Revisit if a measurement ever shows inode-table overhead affecting Firewood's
large-file throughput. The consequence of being wrong in this direction is
1% of a volume; in the other it is a filesystem that fails at 60% full.

Status: not yet run to completion on either machine.

### Where files live

| Path | Device | Holds |
| --- | --- | --- |
| `/` | SATA system disk, 98 GB | the OS |
| `/home` | own volume on the root volume group | source checkouts, dotfiles |
| `/mnt/nvme/<user>` | striped NVMe array | databases, chain state, build artefacts, caches |

Home directories get their own logical volume via
[`setup-home.sh`](setup-home.sh). Otherwise they share the 98 GB root
filesystem, which a couple of Rust target directories can fill; the root
volume group has terabytes unallocated.

Source code lives in home because that keeps it out of the instance, which is
disposable, and because source files are not what the NVMe array is for.
`target/`, the sccache directory and the Go build cache are hot and large, so
the session environment redirects them to `/mnt/nvme/<user>` through
`CARGO_TARGET_DIR`, `SCCACHE_DIR` and `GOCACHE`. People work in `~/src/...`
and the I/O-heavy parts land on the fast disk without anyone arranging it.

### Accounts

Local accounts are created by hand on each machine and added to the `firewood`
group, which grants write access to `/mnt/nvme`.

This is deliberate. Wiring local accounts to Okta requires ongoing involvement
from the security team, whose review cycle is slow and careful. With two
machines and a small team, manual accounts cost less than that process.
Revisit if either number grows.

### Access plumbing

Three Cloudflare tunnels:

| Hostname | Forwards to |
| --- | --- |
| `ethchallenge.avax-dev.network` | `snoopy` sshd |
| `ethchallenge2.avax-dev.network` | `linus` sshd |
| `ethchallenge3.avax-dev.network` | PiKVM, `https://localhost` |

Both hosts trust Cloudflare's SSH certificate authority. Cloudflare mints a
short-lived certificate per connection and the host verifies it against that
CA, so no per-user keys exist on the machines.

The PiKVM's port numbering is inverted relative to the DNS names: port 1 is
`linus`, port 2 is `snoopy`. Correcting it needs either physical access to
re-cable or a Cloudflare DNS change. Both are slow, so it stands as is and is
documented in the README.

### Keeping the machines alike

The hardware already differs (CPU bin, NVMe generation). Software should not
add more divergence: keep the package and snap sets the same on both. Watch
for extras pulled in by the Ubuntu installer, which are easy to select by
accident and show up later as unexplained differences.

[PACKAGES.md](PACKAGES.md) records what is installed beyond a vanilla Ubuntu
26.04 server install. Update it whenever something is added, and diff the two
machines against each other after any change.

### Session images

Not implemented. The intent is that each session runs inside an instance that
starts from a known state and is discarded when the session ends.

Shape:

- LXD system containers, one ephemeral instance per session. Reserve
  `lxc launch --vm` for work that needs its own kernel, since a VM costs I/O
  fidelity. LXD is used because it is already installed; Incus is packaged in
  universe, but the two manage the same kernel primitives and should not share
  a host. `lxd-to-incus` exists if that changes.
- [`provision-session.sh`](provision-session.sh) installs the tools, mirroring
  the list in `.devcontainer/features/firewood-tools/install.sh` so a session
  and a devcontainer offer the same thing. `.devcontainer/` cannot be reused
  directly: it is an OCI image assembled from devcontainer features, while a
  system container boots systemd and behaves like a machine. Neither pins
  versions, so the two will drift; a check that compares them is part of the
  work.
- The script is applied to a base image, which is then baked with
  `lxc publish`. Baking keeps session startup at seconds rather than
  reinstalling toolchains per login.
- The image carries no session account. `fw-session` creates one per instance
  matching the host user's name, uid and gid, so one image serves everyone.
  A fixed account baked into the image would mean depending on a particular
  uid being free, and would leave `~` inside the session pointing somewhere
  other than the home directory mounted from the host. Because the uid
  matches, the daemon's own idmap applies unchanged and no `raw.idmap`
  override is needed.
- The baked toolchains stay root-owned and read-only. `CARGO_TARGET_DIR`,
  `CARGO_INSTALL_ROOT`, `GOPATH` and the caches all point into the user's data
  directory, so nothing in the image needs to be writable by an account that
  does not exist when it is built.
- Confined projects get their own image store by default, so a project must be
  set `features.images false` to see the image published in the default
  project. Without that, each user needs a private copy of a multi-gigabyte
  image and build-once-copy-once stops meaning anything.
- Build the image once and copy it to the other machine. Running the same
  provisioning script on both hosts does not produce the same image: package
  managers fetch whatever is current at build time. The source is
  reproducible as a process, not as an artifact. The built image is the
  artifact of record, aliased by date, with the previous one kept for
  rollback.
- Attach the user's host directory `/mnt/nvme/<user>` as a disk device. The
  instance is disposable; the data is not. A multi-hour state fetch must not
  die with the session.
- Enter the session through `ForceCommand` in `sshd_config` under a
  `Match Group` block, excluding an admin group so the host stays directly
  reachable.

A published image goes into LXD's own image store, addressed by fingerprint
with an alias attached for convenience, rather than being a file anyone
manages. It lands in two places: the image records under
`/var/snap/lxd/common/lxd/images` on the SATA root disk, and, once an instance
uses it, an unpacked volume in the `nvme` pool on the array. Instance
filesystems are therefore on the fast disk and only the archives are not. The
root filesystem is 98 GB, so delete superseded images rather than letting them
accumulate.

The store is per host, which is why the image has to be exported and imported
rather than published twice. An exported tarball is also the only copy that
survives an LXD reinstall.

Problems to solve first:

- Unprivileged instances shift UIDs. A bind-mounted host directory appears as
  `nobody:nogroup` inside unless idmapped mounts or a `raw.idmap` entry is
  configured.
- Docker's default seccomp profile blocks the `io_uring` syscalls. Firewood
  sets `cfg(io_uring)` on Linux (`storage/build.rs`), so whichever runtime is
  chosen must be verified to permit them.
- Neither containers nor VMs reset page cache, CPU thermal and turbo state, or
  the NVMe drives' SLC cache and wear. Repeatable sessions give a repeatable
  software environment, not a repeatable machine. Benchmark repeatability also
  needs host-level resets: `drop_caches`, `fstrim`, and a pinned CPU governor.
- The two hosts cannot reach each other. They sit behind separate Cloudflare
  tunnels with no path between them, so image transfer goes through a
  workstation with `lxc image export` and `import`. `lxc image copy` is
  not available.

Striping all four NVMe devices into one volume group means a future VM session
cannot be given a dedicated disk and would use a disk image on the shared
filesystem instead. Accepted, on the basis that containers are the default and
VMs the exception.

### Toolchains

The hosts carry Nix 2.34.3 and nothing else. No language toolchains are
installed on them.

Toolchains belong in the session image above. Host-installed toolchains are
what makes sessions non-repeatable: each person ends up with whatever they
installed, and the hosts drift from the image and from each other. A bare host
has nothing to drift.

Until the session image exists, `nix develop` against `ffi/flake.nix` provides
pinned toolchains without installing anything on the host.

## Runbooks

Command sequences only. See [Design](#design) for why any of it is shaped this
way.

Bringing up a machine from scratch, in order. Each step verifies before the
next depends on it:

1. [Configure the NVMe array](#configure-the-nvme-array)
2. [Move /home onto its own volume](#move-home-onto-its-own-volume)
3. [Initialise LXD](#initialise-lxd) onto that array
4. [Build a session image](#build-a-session-image-and-push-it-to-both-machines)
5. [Add a user](#add-a-user) for each team member

Steps 1 to 3 are per machine. Step 4 runs on one machine and the image is
copied to the other. Step 5 is per machine, per person.

### Configure the NVMe array

Once per machine. Destroys all data on the NVMe devices.

```bash
sudo bash infra/onprem/setup-nvme.sh --dry-run   # check the device list
sudo bash infra/onprem/setup-nvme.sh
```

If a device is already in use the script names what is on it and stops. The
NVMe devices shipped carrying an mdadm array with an ext3 filesystem, so this
is the expected first result. Inspect before deciding it is disposable:

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
Zeroing the RAID superblock is the part that matters: without it the array
reassembles on the next boot and takes the disks back from LVM. The script
also warns if `/etc/mdadm/mdadm.conf` still names an array.

`--help` lists the options. Reboot afterwards and confirm the mount returns:

```bash
sudo reboot
df -hT /mnt/nvme && sudo lvs firewood
```

### Record installed packages

On each machine, then paste each host's output into its section of
[PACKAGES.md](PACKAGES.md):

```bash
bash infra/onprem/inventory-packages.sh
```

Diff the two to confirm the machines still agree.

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
start sessions with `fw-session` and need no checkout of this repository; a
script under one person's home directory is not readable by other accounts
anyway.

It then sets up their confined LXD project. The project is created by LXD's
multi-user daemon the first time that user runs any `lxc` command, so the
script triggers that itself rather than waiting for their first login, and
then sets three things on it:

| Setting | Why |
| --- | --- |
| `features.images false` | see the session image published in the default project, instead of needing a private copy |
| `restricted.devices.disk allow` | permit disk devices at all |
| `restricted.devices.disk.paths` | confine those sources to the user's own directory |

The third is not optional. `allow` with an empty paths list permits any host
path, which is host root by another route.

Password login stays disabled, since Cloudflare authenticates before the
connection reaches the machine. An account that needs `sudo` therefore also
needs a password or a `NOPASSWD` rule, and the script says so when `--sudo` is
used.

When a new per-user setup step appears, add it to `add-user.sh` so one script
stays the complete answer.

### Move /home onto its own volume

Once per machine, with nobody logged in. The script refuses to run otherwise,
since copying home while someone is writing to it loses their work.

```bash
sudo bash infra/onprem/setup-home.sh --dry-run
sudo bash infra/onprem/setup-home.sh
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

```bash
sudo mkdir -p /mnt/nvme/lxd
sudo lxd init --preseed < infra/onprem/lxd-init.yaml
lxc storage list          # expect pool 'nvme' with source /mnt/nvme/lxd
```

### Build a session image and push it to both machines

Runs on one machine; the image is copied to the other. Untested so far: the
steps below have not been run end to end.

Build on `snoopy`:

```bash
TAG=firewood-session-$(date +%Y%m%d)
lxc launch ubuntu:26.04 build-tmp
lxc file push infra/onprem/provision-session.sh build-tmp/root/
lxc exec build-tmp -- bash /root/provision-session.sh
```

The script defaults to `azure.archive.ubuntu.com` and measures it before
installing anything, warning if it is under 1 MB/s and continuing regardless.
Most Ubuntu mirrors measured in the hundreds of bytes per second from these
machines while the host link ran at 87 MB/s, which is why the default is not
the usual one. That is probably a symptom of resolute being newly released
rather than a lasting property of those mirrors, so treat the default as a
starting point and re-measure if provisioning drags.

If the warning fires, compare candidates from the host and pass the winner
with `--apt-mirror`:

```bash
for m in archive.ubuntu.com us.archive.ubuntu.com azure.archive.ubuntu.com \
         mirrors.kernel.org; do
  printf '%-28s ' "$m"
  curl -o /dev/null -w '%{speed_download} B/s\n' -s --max-time 20 \
    "http://$m/ubuntu/dists/resolute/main/binary-amd64/Packages.gz" || echo fail
done
```

`--keep-apt-mirror` leaves the image's own sources alone.

Only apt is covered. `rustup`, `go.dev` and GitHub releases are separate
sources, so a stall under the `Rust` or `Cargo tools` step is something else.

That last command prints `rustup show`, `go version`, `sccache --version` and
`just --version` when it succeeds. Check them before publishing, since a
half-provisioned image is worse than none. Then:

```bash
lxc stop build-tmp
lxc publish build-tmp --alias "$TAG"
lxc delete build-tmp
```

Point the stable alias at it:

```bash
lxc image alias delete firewood-session || true
lxc image alias create firewood-session "$(lxc image info "$TAG" | awk '/Fingerprint/ {print $2}')"
```

Copy to `linus`. The hosts cannot reach each other, so the image goes through
your workstation:

```bash
# on snoopy
lxc image export "$TAG" "/tmp/$TAG"

# on your workstation
scp snoopy:/tmp/$TAG.tar.gz .
scp $TAG.tar.gz linus:/tmp/

# on linus
lxc image import "/tmp/$TAG.tar.gz" --alias "$TAG"
```

Repeat the alias step on `linus`. Keep the previous image for rollback.

## Open items

- Session images, per [Session images](#session-images) above.
- Run `setup-nvme.sh` on both machines.
- Observability. Not set up, and needed.
  `benchmark/setup-scripts/install-grafana.sh` is the EC2 precedent: Grafana on
  port 3000, coreth metrics on 6060. Decide whether to run it per host or
  centrally.
- C-Chain state has no agreed location; each user currently decides. At 1 Gbps
  a full fetch takes hours, so a shared read-only copy under `/mnt/nvme` is
  worth considering.
- Reservation is advisory. No mechanism enforces or records who holds a
  machine.
- PACKAGES.md has no inventory yet. Run `inventory-packages.sh` on both
  machines and fill it in.
