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
- `bytes-per-inode` is 2097152, matching
  `benchmark/setup-scripts/build-environment.sh`, which provisions the EC2
  equivalent. That figure suits LevelDB's many small files.

Status: not yet run on either machine.

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
- The image is user-agnostic: it carries one `dev` account at uid 1000 and the
  host maps the session owner onto it, so a single image serves everyone.
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
2. [Initialise LXD](#initialise-lxd) onto that array
3. [Build a session image](#build-a-session-image-and-push-it-to-both-machines)
4. [Add a user](#add-a-user) for each team member

Steps 1 and 2 are per machine. Step 3 runs on one machine and the image is
copied to the other. Step 4 is per machine, per person.

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

Password login stays disabled, since Cloudflare authenticates before the
connection reaches the machine. An account that needs `sudo` therefore also
needs a password or a `NOPASSWD` rule, and the script says so when `--sudo` is
used.

When a new per-user setup step appears, add it to `add-user.sh` so one script
stays the complete answer.

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
