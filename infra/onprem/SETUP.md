# Setting Up the On-Prem Servers

How `snoopy` and `linus` are built and maintained. For day-to-day use, see
[README.md](README.md).

This document is in two halves. [Design](#design) explains how things are set
up and why, for anyone who needs to understand what they are dealing with.
[Runbooks](#runbooks) is the command sequences on their own, for when you
already understand and just need the steps.

## Design

These on-prem machines are configured with these goals in mind:

- Predictable performance for benchmarking.  Minimize sharing and virtual layers between the code and the hardware.
- Cheap and easy reset of environment -- feel free to change things and bring it back to a known state with one command.
- Minimal cost of administration and networking
- Maximum security -- zero trust access.
  
### Storage

[`setup-nvme.sh`](setup-nvme.sh) (run only once per machine) stripes every empty NVMe device into one LVM
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

`bytes-per-inode` fixes the inode count for the life of the filesystem.

The EC2 script uses 2 MB per inode, which on a 7.3 TB volume yields 3.6 M
inodes and saves roughly 116 GB of inode tables against the ext4 default, an insignificant savings of about 1% which would come at the cost of not being able to hold the tens of thousands of small files that we generate when building into Rust target directories.

Revisit if a
measurement shows inode-table overhead affecting Firewood's large-file
throughput. 

### Where files live

| Path | Device | Holds |
| --- | --- | --- |
| `/` | SATA system disk, 98 GB | the OS |
| `/home` | own volume on the root volume group | source checkouts, dotfiles |
| `/mnt/nvme/<user>` | striped NVMe array | databases, chain state, build artefacts, caches |

Home directories get their own logical volume via
[`setup-home.sh`](setup-home.sh) (run only once per machine).  Otherwise they would share the 98 GB root
filesystem, which a couple of Rust target directories can fill; the root
volume group has terabytes unallocated, and expansion of home space is trivial when/if needed.

Source code lives in home because it does not need performance.
`target/`, the sccache directory and the Go build cache are hot and large, so
the session environment redirects them to `/mnt/nvme/<user>` through
`CARGO_TARGET_DIR`, `SCCACHE_DIR` and `GOCACHE`. People work in `~/...`
and the I/O-heavy parts land on the fast disk without anyone arranging it.

The environment variables are set by default for each user.

### Accounts

Local accounts are created for each user on each machine and added to the `firewood`
group, which grants write access to `/mnt/nvme`.  This has to be done on each machine separately because, in a zero-trust environment, and without erecting excessive infrastructure complexity, the machines cannot communicate directly. 

Accounts are local and not synchronized with the company's authorization fabric (Okta) to minimize adminstration burden, failure modes, and involvement from the security team. With two
machines and a small team, manual accounts cost less than that process.
Revisit if either number grows.

### Access plumbing

In a zero-trust environment we rely on Cloudflare's zero trust infrastucture for access, vie these three Cloudflare tunnels:

| Hostname | Forwards to |
| --- | --- |
| `ethchallenge.avax-dev.network` | `snoopy` sshd |
| `ethchallenge2.avax-dev.network` | `linus` sshd |
| `ethchallenge3.avax-dev.network` | PiKVM, `https://localhost` |

The first two tunnels are meant for regular use.  The third is for tasks that require access to the consoles, e.g, for updating the firmware or the operating system.  Both hosts trust Cloudflare's SSH certificate authority. Cloudflare mints a
short-lived certificate per connection and the host verifies it against that
CA, so no per-user keys exist on the machines.

The tunnels themselves, the DNS records, and the certificate authority are
managed by the security team; we have no access to that configuration. So a
machine rebuilt from scratch needs a request to them for the tunnel and the
CA, and cannot be brought back onto the network without it. Everything else
here is reproducible from this repository.

The PiKVM's port numbering is inverted relative to the DNS names: port 1 is
`linus`, port 2 is `snoopy`. Correcting this requires physical access to the IDF; we will do it as convenient.  

Besides each user name for each member of the team, we have these users:

| User Name | Where | What for |
| --- | --- | --- |
| firewood | snoopy, linus | `sudo` user on hosts |
| root | PiKVM | Management of Linux on switch |
| admin | PiKVM | Management of web UI credentias on switch |

The passwords for this users are on `1password`.  Ask someone who knows.

### Keeping the machines alike

The hardware already differs (CPU bin, NVMe generation). Software should not
add more divergence: keep the package and snap sets the same on both. Watch
for extras pulled in by the Ubuntu installer, which are easy to select by
accident and show up later as unexplained differences.

[PACKAGES.md](PACKAGES.md) records what is installed beyond a vanilla Ubuntu
26.04 server install. Update it whenever something is added, and diff the two
machines against each other after any change.

### Session images

Logging in over SSH attaches to a persistent per-user container. These containers cum sessions are managed by the `fw-session` utility.  A session is created (or joined) upon user login, from a shared pre-built image, and removed only when its owner runs
`fw-session destroy`.

That persistence replaced an earlier design in which the container was
discarded on logout. The reason for the change: a dropped connection would
otherwise kill a multi-hour run.
The cost is that a long-lived instance drifts from the image, which we mitigate by having the command `fw-session`
report when a newer image exists and work is worth starting
from `fw-session recreate`.

Why containers: a cheap, predictable reset to a known state. The
alternative considered was no containers — `nix develop` against
`ffi/flake.nix` for toolchains, `tmux` on the host for persistence — which
gives nearly the same daily workflow for a fraction of the machinery. It was
rejected because it offers no reset, and because nothing then stops the host
drifting as people install things; adding unnecessary variables that might affect performance measurements over time. Isolation between users is a side effect, not the
motivation: this is a trusted team.

This stands provisionally until I/O inside a session is measured against bare
metal (see below). If they differed materially, benchmarks would belong on the host and
sessions would only be used for development, if at all.

- LXD system containers, one per user. `lxc launch --vm` is available for work
  needing its own kernel, at the cost of I/O fidelity. LXD rather than Incus
  because it is installed by the Ubuntu bootstrap workflow.
- LXD's multi-user daemon gives each member of the `firewood` group a confined
  project of their own, created on first use and named `user-<uid>`. Members of
  the `lxd` group would instead get full administrative access, which is
  equivalent to root on the host.
- Confinement is what makes root inside a session safe. A confined user can
  attach disk devices only with sources under the prefixes in
  `restricted.devices.disk.paths` (part of the LXD configuration), set by `add-user.sh` (our script) to their home and data
  directories. 
- [`provision-session.sh`](provision-session.sh) (our script, executed only occassionally to build an image) installs the toolchains,
  matching `.devcontainer/features/firewood-tools/install.sh`. Neither pins
  apt, rustup or cargo-binstall versions, so the two drift. `.devcontainer/` cannot be reused
  directly: it is an OCI image assembled from devcontainer features, while a
  system container boots `systemd`.
- The image is shared by all accounts/sessions.  `fw-session` configures each session to match the user's name, uid and gid, so `~` inside the session is the
  same path as outside on the bare machine, where their home is mounted from. The baked toolchains
  stay root-owned; `CARGO_TARGET_DIR`, `CARGO_INSTALL_ROOT`, `GOPATH` and the
  caches point into the user's data directory.
- Build the image once (using `provision-session.sh`) and copy it to the other machine. The same script run
  twice does not produce the same image, and the hosts cannot reach each other,
  so transfer is `lxc image export`/`import` through an external location reachable from both.
- Entry into the session upon login is through `/etc/profile.d/fw-session.sh`, installed by `add-user.sh`. It is bypassed by console logins, accounts with no data directory, and members of the `sudo`
  group, so administrators login into a host shell: the runbooks below all operate on
  the host, and running one inside a session would configure the container.

Still unverified:

- Whether `io_uring` works in an unprivileged container. Firewood sets
  `cfg(io_uring)` on Linux (`storage/build.rs`), so a session may exercise a
  different I/O path than production without anyone noticing.  These are _system_ containers and all evidence suggests they do support `io_uring` but we have not yet verified.  Revisit and edit this item when we do.
- Whether a benchmark inside a session matches one on bare metal. This is the
  criterion the exercise exists to serve and nothing has measured it. A `fio`
  run and a short re-execution, host against session, would settle it.

Containers do not reset page cache, CPU thermal and turbo state, or the NVMe
drives' SLC cache and wear. Sessions give a repeatable software environment,
not a repeatable machine; true benchmark repeatability also needs host-level resets.

Striping all four NVMe devices into one volume group means a hypothetical VM session would not have a dedicated partition and would be limited to a disk image on the shared filesystem.
Accepted: containers are the default and VMs the exception.

### Apt mirrors

The session image build defaults to `azure.archive.ubuntu.com`, not the usual
mirror, only because at the time we configured it the standard mirrors offered ridiculuously poor bandwidth in the order of hundreds of _bytes_ per second. Treat the default as a starting
point: `provision-session.sh` measures whatever it is given and warns under
1 MB/s. Only apt is covered; `rustup`, `go.dev` and GitHub releases are
separate, so a stall in the `Rust` or `Cargo tools` step requires a different adjustment.

To compare candidates from a host:

```bash
for m in archive.ubuntu.com azure.archive.ubuntu.com mirrors.kernel.org; do
  printf '%-28s ' "$m"
  curl -o /dev/null -w '%{speed_download} B/s\n' -s --max-time 20 \
    "http://$m/ubuntu/dists/resolute/main/binary-amd64/Packages.gz" || echo fail
done
```

### Toolchains

The hosts carry Nix 2.34.3 and nothing else. No language toolchains are
installed on them.

Toolchains are in the repeatable session image. 

`nix develop` against `ffi/flake.nix` provides pinned toolchains on the host
for work that should not run in a session.

## Runbooks

Bringing up a machine from scratch, in order. Each step verifies before the
next depends on it:

1. Install OS through the KVM switch (not covered here).  Pay attention to what packages we rely upon, e.g, `lxc`. `PACKAGES.md` may be useful in this.
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

If a device is already in use the script names what is on it and stops. Inspect before deciding whether it is disposable:

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

If you installed the OS with `/home` on a separate partition, congratulations, you can skip this step.  Otherwise, you run through this once per machine, with nobody else logged in. The script refuses to run
otherwise, since copying home while someone is writing to it risks inconsistent state.

Two wrinkles, because this step probably replaces the filesystem this git repository is checked out to: 1) Run it from outside `/home`; and 2) run a copy of the script from `/tmp`, since bash reads a script as
it executes and the original is about to be masked by the new mount:

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

```bash
sudo mkdir -p /mnt/nvme/lxd
sudo lxd init --preseed < infra/onprem/lxd-init.yaml
lxc storage list          # expect pool 'default', source /mnt/nvme/lxd
```

Then hand the `firewood` group confined access. This is a snap option, not LXD
configuration, so the preseed cannot set it, and without it no confined
projects exist and nobody can run `lxc` at all:

```bash
sudo snap set lxd daemon.user.group=firewood
snap get lxd daemon.user.group      # expect: firewood
```

Do this before adding users. Nobody should be in the `lxd` group, which grants
full administrative access to LXD and is equivalent to root on the host.

### Build a session image and push it to both machines

Runs on one machine; the image is copied to the other.

Every command here uses `sudo`. Once `daemon.user.group` is set, a bare `lxc`
from an administrator's account operates in their own confined project, which
has `features.images false` and cannot hold images. The image has to live in
the default project, which is what confined projects share from.

Build on one machine (e.g, `snoopy`):

```bash
TAG=firewood-session-$(date +%Y%m%d)
sudo lxc launch ubuntu:26.04 build-tmp
sudo lxc file push infra/onprem/provision-session.sh build-tmp/root/
sudo lxc exec build-tmp -- bash /root/provision-session.sh
```

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
sudo lxc image export "$TAG" "/tmp/$TAG"
echo "$TAG"           # note this; the other machine needs it
```

Then carry the tarball to the other machines (e.g, `linus`):

```bash
# on your workstation
scp snoopy:/tmp/<tag>.tar.gz .
scp <tag>.tar.gz linus:/tmp/

# on linus, with TAG set to the same value
sudo lxc image import "/tmp/$TAG.tar.gz" --alias "$TAG"
sudo lxc image alias create firewood-session \
    "$(sudo lxc image info "$TAG" | awk '/Fingerprint/ {print $2}')"
```

Keep the previous image for rollback.

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

### Record installed packages

On each machine, then paste each host's output into its section of
[PACKAGES.md](PACKAGES.md):

```bash
bash infra/onprem/inventory-packages.sh
```

Diff the two to confirm the machines still agree.

## Open items

- **No backup, and `/mnt/nvme` has no redundancy.** It is a four-way stripe:
  one drive failing loses every user's data and the LXD pool with it. `/home`
  is a single volume on a single SSD. Either accept that explicitly in the
  README or arrange something.
- **No quotas.** One user filling `/mnt/nvme` stops every session on the
  machine. The `dir` storage driver offers none.
- **Benchmark repeatability might need host-level resets** — `drop_caches`,
  `fstrim`.
- **No offboarding.** Removing someone leaves their LXD project, their
  `lxdbr-<uid>` bridge, their instance and their data directory behind.
- **C-Chain state has no agreed location.** At 1 Gbps a full fetch takes hours,
  so a shared read-only copy under `/mnt/nvme` is worth considering.
- **Reservation is advisory.** Nothing records or enforces who holds a machine.

