#!/bin/bash
# Prints the packages installed on this machine, as Markdown, for pasting into
# PACKAGES.md.
#
# `apt-mark showmanual` lists packages that were asked for rather than pulled
# in as dependencies. It includes the set the Ubuntu installer selects, so the
# output is not purely what was added by hand; compare against a vanilla
# install, or against the other machine, to see the difference.
#
# Run on both machines and diff the output. The two should agree.
set -o errexit
set -o nounset
set -o pipefail

echo "### $(hostname)"
echo ""
echo "Ubuntu $(lsb_release -rs), kernel $(uname -r). Collected $(date -u +%Y-%m-%d)."
echo ""

echo "Manually installed apt packages:"
echo ""
echo '```text'
apt-mark showmanual | sort
echo '```'
echo ""

echo "Snaps:"
echo ""
echo '```text'
snap list | tail -n +2 | awk '{print $1, $2}' | sort
echo '```'
