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

### Toolchains

Both machines carry Go 1.26.0 and Nix 2.34.3.

Rust is 1.93.1, built from a source tarball, which is below the workspace MSRV
of 1.94.0. `--all-features` needs 1.94.1, because the AWS SDK crates behind
`fwdctl`'s `launch` feature require it. Firewood does not build until this is
raised.

### Session images

Not implemented. The intent is that each session runs inside an instance that
starts from a known state and is discarded when the session ends.

Shape:

- Incus system containers, one ephemeral instance per session. Reserve
  `incus launch --vm` for work that needs its own kernel, since a VM costs I/O
  fidelity.
- The instance definition mirrors the toolchain versions `.devcontainer/`
  pins. `.devcontainer/` itself cannot be reused directly: it is an OCI image
  assembled from devcontainer features, while a system container boots systemd
  and behaves like a machine. Two places pinning Rust and Go versions will
  drift, so a check that compares them is part of the work.
- Provisioning source lives in this directory and is applied to a base image,
  which is then baked with `incus publish`. Baking keeps session startup at
  seconds rather than reinstalling toolchains per login.
- Build the image once and copy it to the other machine. Running the same
  provisioning script on both hosts does not produce the same image: `apt`,
  `rustup`, and `nix` fetch whatever is current at build time. The source is
  reproducible as a process, not as an artifact. The built image is the
  artifact of record, aliased by date, with the previous one kept for
  rollback.
- Attach the user's host directory `/mnt/nvme/<user>` as a disk device. The
  instance is disposable; the data is not. A multi-hour state fetch must not
  die with the session.
- Enter the session through `ForceCommand` in `sshd_config` under a
  `Match Group` block, excluding an admin group so the host stays directly
  reachable.

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
  workstation with `incus image export` and `import`. `incus image copy` is
  not available.

Striping all four NVMe devices into one volume group means a future VM session
cannot be given a dedicated disk and would use a disk image on the shared
filesystem instead. Accepted, on the basis that containers are the default and
VMs the exception.

## Runbooks

Command sequences only. See [Design](#design) for why any of it is shaped this
way.

### Configure the NVMe array

Once per machine. Destroys all data on the NVMe devices.

```bash
sudo bash infra/onprem/setup-nvme.sh --dry-run   # check the device list
sudo bash infra/onprem/setup-nvme.sh
```

`--help` lists the options. Reboot afterwards and confirm the mount returns:

```bash
sudo reboot
df -hT /mnt/nvme && sudo lvs bench
```

### Add a user

On each machine:

```bash
sudo adduser <username>
sudo usermod -aG firewood <username>
```

Group membership takes effect at their next login.

### Build a session image and push it to both machines

Not yet implemented; the provisioning script does not exist. Recorded so the
shape is agreed before it is written.

Build on `snoopy`:

```bash
TAG=firewood-session-$(date +%Y%m%d)
incus launch images:ubuntu/26.04 build-tmp
incus file push infra/onprem/provision-session.sh build-tmp/root/
incus exec build-tmp -- bash /root/provision-session.sh
incus stop build-tmp
incus publish build-tmp --alias "$TAG"
incus delete build-tmp
```

Point the stable alias at it:

```bash
incus image alias delete firewood-session || true
incus image alias create firewood-session "$(incus image info "$TAG" | awk '/Fingerprint/ {print $2}')"
```

Copy to `linus`. The hosts cannot reach each other, so the image goes through
your workstation:

```bash
# on snoopy
incus image export "$TAG" "/tmp/$TAG"

# on your workstation
scp snoopy:/tmp/$TAG.tar.gz .
scp $TAG.tar.gz linus:/tmp/

# on linus
incus image import "/tmp/$TAG.tar.gz" --alias "$TAG"
```

Repeat the alias step on `linus`. Keep the previous image for rollback.

## Open items

- Session images, per [Session images](#session-images) above.
- Raise Rust to at least 1.94.1 on both machines.
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
