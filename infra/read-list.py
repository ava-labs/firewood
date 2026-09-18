#!/usr/bin/env python3
"""Print one item per line from a list in a YAML manifest.

Usage: read-list.py MANIFEST KEY [KEY ...]

Keys walk into nested mappings; the last one must name a list of strings.
Callers read the output with `mapfile -t`, which splits on newlines, so an item
containing one is rejected rather than silently becoming two.
"""

import sys

import yaml


def main(argv):
    if len(argv) < 3:
        sys.exit(f"usage: {argv[0]} MANIFEST KEY [KEY ...]")

    manifest, keys = argv[1], argv[2:]
    path = ".".join(keys)

    try:
        with open(manifest, encoding="utf-8") as handle:
            node = yaml.safe_load(handle) or {}
    except OSError as err:
        sys.exit(f"read-list: {err}")
    except yaml.YAMLError as err:
        sys.exit(f"read-list: {manifest} is not valid YAML: {err}")

    for key in keys:
        if not isinstance(node, dict) or key not in node:
            sys.exit(f"read-list: {manifest} has no {path}")
        node = node[key]

    if not isinstance(node, list):
        sys.exit(f"read-list: {manifest}: {path} is not a list")

    for item in node:
        if not isinstance(item, str) or "\n" in item:
            sys.exit(f"read-list: {manifest}: {path} must hold single-line strings")
        print(item)


if __name__ == "__main__":
    main(sys.argv)
