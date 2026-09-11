# On-Prem Servers

Two Linux servers, `snoopy` and `linus`, in the IDF at the Ava Labs office.
Shared by the Firewood team. Internal only.

Most of what has run on them so far is performance benchmarking, in particular
C-Chain re-execution from genesis. They are not reserved for that. Any work
that needs real hardware, large disks, or long runtimes is a fair use.

For how the machines are built and maintained, see
[ADMINISTRATION.md](ADMINISTRATION.md).

## Hardware

| | snoopy | linus |
| --- | --- | --- |
| CPU | Intel i9-14900K, 24C/32T | Intel i9-14900KS, 24C/32T |
| RAM | 61 GiB | 62 GiB |
| System disk | Samsung SSD 870, 7.3 TB, SATA | Samsung SSD 870, 7.3 TB, SATA |
| NVMe | 4 x 1 TB Crucial T700, PCIe Gen5 | 4 x 2 TB Crucial T500, PCIe Gen4 |
| Network | 1 Gbps | 1 Gbps |
| OS | Ubuntu 26.04.1 LTS, kernel 7.0.0-31, no GUI | Ubuntu 26.04.1 LTS, kernel 7.0.0-31, no GUI |

The two machines are not identical. `linus` has the faster CPU bin and twice
the NVMe capacity; `snoopy` has the faster NVMe generation. Do not compare a
measurement taken on one against a measurement taken on the other. Run both
sides of an A/B on the same host.

## Access

There are two ways in: SSH through a Cloudflare tunnel, and the PiKVM console.
Neither machine is reachable any other way.

### SSH

Install `cloudflared` locally, then add to `~/.ssh/config`:

```text
Host snoopy
  HostName ethchallenge.avax-dev.network

Host linus
  HostName ethchallenge2.avax-dev.network

Host snoopy linus
  User <your-username>
  ProxyCommand cloudflared access ssh --hostname %h
```

Use an absolute path in `ProxyCommand` if `cloudflared` is not on your `PATH`
(`/opt/homebrew/bin/cloudflared` on Apple silicon). Then `ssh snoopy`.

Authentication uses Cloudflare short-lived certificates. Cloudflare
authenticates you, issues a transient certificate, and the host verifies it
against Cloudflare's certificate authority. There is no second login on the
host and no SSH key to install or rotate.

### Console

<https://ethchallenge3.avax-dev.network> reaches the PiKVM: console video,
power control, and virtual media for reinstalling an OS.

The KVM port numbering is the reverse of the DNS numbering:

| KVM port | Machine | DNS |
| --- | --- | --- |
| 1 | `linus` | `ethchallenge2` |
| 2 | `snoopy` | `ethchallenge` |

Confirm which machine is on screen before power-cycling anything.

## Accounts

Every team member has their own account on each machine, created by hand. Ask
an administrator for one.

The shared `firewood` account exists for machine management. Do not use it for
routine work.

## Where to put your data

`/` lives on the SATA system disk. Working data belongs on the NVMe array
mounted at `/mnt/nvme`.

Your own directory there is `/mnt/nvme/$USER/firewood`, linked from your home
directory as `~/firewood`. Both are created when your account is set up. If
`~/firewood` is missing, ask an administrator rather than creating it by hand,
so the ownership and group are right.

The NVMe array is not configured yet on either machine. See
[ADMINISTRATION.md](ADMINISTRATION.md).

## Reserving a machine

Reservation is advisory and voluntary. Tell the team before starting long or
performance-sensitive work, and check whether someone else has claimed the
machine first. If both are busy, wait rather than sharing: concurrent work
distorts anyone's measurements. Nothing enforces this.

## Constraints to plan around

- The link is 1 Gbps. Fetching C-Chain state over the network takes hours.
  Keep a local copy and reuse it instead of re-syncing per run.
- Server installs, no GUI.
