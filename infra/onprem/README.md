# On-Prem Servers

Two Linux servers, `snoopy` and `linus`, in the IDF at the Ava Labs office.
Shared by the Firewood team. Internal only.

Most of what has run on them so far is performance benchmarking, in particular
C-Chain re-execution from genesis. They are not reserved for that. Any work
that needs real hardware, large disks, or long runtimes is a fair use.

For how the machines are built and maintained, see
[SETUP.md](SETUP.md).

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
mounted at `/mnt/nvme`. The link is 1 Gbps, so fetching C-Chain state takes
hours: keep a local copy rather than re-syncing per run.

Your own directory there is `/mnt/nvme/$USER/firewood`, linked from your home
directory as `~/firewood`. Both are created when your account is set up; if
`~/firewood` is missing, ask an administrator.

`/mnt/nvme` is a stripe across four drives with no redundancy, and there are
no backups. Anything you cannot regenerate belongs in git or off the machine.

## Working in a session

Logging in over SSH puts you inside your own container, created on first use
from a shared image with the toolchains already installed. You have root
inside it; you have no privilege on the host.

It persists until you remove it, so logging back in returns you to it, and
several terminals can be attached at once.

```bash
fw-session            # enter your session, creating it if needed
fw-session status     # is it running, and is the image current
fw-session destroy    # remove it; your files are untouched
fw-session recreate   # rebuild from the current image
```

Your dotfiles are your own. Nothing here writes to your home directory and
sessions do not depend on it, so edit freely; because home is mounted, the
same files apply inside and out. Prepend to `PATH` rather than replacing it,
or you will hide the session's toolchain.

Typing `exit` inside a session returns you to a plain host shell rather than
logging you out. That is where `fw-session destroy` is run. You can also do it
without entering the session at all, since non-interactive commands bypass the
login hook:

```bash
ssh snoopy fw-session status
```

**Long runs need `tmux`.** The container survives a disconnect, but your shell
does not: anything running in the foreground dies with the connection. Start
benchmarks inside `tmux` so they keep going.

```bash
tmux new -s bench
# ... start the run, then detach with ctrl-b d
tmux attach -t bench
```

What survives what:

| | Disconnect | Host reboot | `destroy` |
| --- | --- | --- | --- |
| Files in `~` and `/mnt/nvme/$USER` | yes | yes | yes |
| The container and anything installed in it | yes | yes | no |
| Foreground processes | no | no | no |
| Processes under `tmux` | yes | no | no |

Sessions drift from the image as people install things, so anything
benchmark-grade is worth starting with `fw-session recreate`. `fw-session`
says when a newer image has been published, but nothing is enforced.

## Reserving a machine

Reservation is advisory and voluntary. Tell the team before starting long or
performance-sensitive work, and check whether someone else has claimed the
machine first. If both are busy, wait rather than sharing: concurrent work
distorts anyone's measurements. Nothing enforces this.
