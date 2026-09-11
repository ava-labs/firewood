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
| | | |

## Inventory

Not yet collected. Run the script on each machine and paste the output here.

### snoopy

### linus
