# Installed Packages

What is installed on `snoopy` and `linus` beyond a vanilla Ubuntu 26.04 server
install. Kept so the two machines can be compared against each other and
rebuilt from scratch.

Regenerate with [`inventory-packages.sh`](inventory-packages.sh), run on each
machine, and replace the section for that host:

```bash
bash infra/onprem/inventory-packages.sh
```

`apt-mark showmanual` includes what the Ubuntu installer selected, so the raw
output is not purely hand-added packages. Diff the two hosts against each
other: they should agree, and anything that differs is either deliberate or a
mistake worth chasing.

## Added deliberately

Packages installed on purpose, and why. Keep this list curated by hand; the
generated sections below are the raw record.

| Package | Source | Why |
| --- | --- | --- |
| `cloudflared` | apt | SSH access runs through a Cloudflare tunnel |
| `fio` | apt | throughput check in `setup-nvme.sh` |
| `nix-bin` | apt | pinned toolchains without installing them on the host |
| `lxd` | snap | session containers |
| `prometheus` | snap | unconfirmed; see [Unexplained](#unexplained) |
| `aws-cli` | snap | unconfirmed; see [Unexplained](#unexplained) |
| `hwctl` | snap | unconfirmed; see [Unexplained](#unexplained) |

Everything else in the inventory below is part of a vanilla Ubuntu 26.04
server install, or a dependency of one of the above.

## Inventory

### snoopy

Ubuntu 26.04, kernel 7.0.0-31-generic. Collected 2026-09-11.

Manually installed apt packages:

```text
bash
cloudflared
dash
diffutils
efibootmgr
findutils
fio
grep
grub-efi-amd64
grub-efi-amd64-signed
gzip
hostname
init
linux-generic
ncurses-base
ncurses-bin
nix-bin
openssh-server
shim-signed
ubuntu-minimal
ubuntu-server
ubuntu-server-minimal
ubuntu-standard
util-linux
wpasupplicant
```

Snaps:

```text
aws-cli 2.35.21
core20 20260901
core22 20260410
core24 20260410
hwctl 0.11.1
lxd 5.21.7-1018661
prometheus 2.37.0
snapd 2.76.3
```

### linus

Ubuntu 26.04, kernel 7.0.0-31-generic. Collected 2026-09-11.

Manually installed apt packages:

```text
bash
cloudflared
dash
diffutils
efibootmgr
emacs-nox
findutils
fio
grep
grub-efi-amd64
grub-efi-amd64-signed
gzip
hostname
init
linux-generic
ncurses-base
ncurses-bin
nix-bin
openssh-server
shim-signed
ubuntu-minimal
ubuntu-server
ubuntu-server-minimal
ubuntu-standard
util-linux
wpasupplicant
```

Snaps:

```text
aws-cli 2.35.21
core18 20260204
core20 20260410
core22 20260410
core24 20260410
hwctl 0.11.1
lxd 5.21.7-1018661
prometheus 2.37.0
snapd 2.76.3
```
