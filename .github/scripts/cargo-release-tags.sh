#!/usr/bin/env bash

# This script emits all of the local cargo packages and their current versions
# in topological order, so that they can be published to crates.io in the
# correct order. The output is in the format "package-name/vVERSION", one per
# line. When tagging cargo packages, the tag is expected to match the format
# exactly.

set -euo pipefail

METADATA=$(cargo metadata --locked --format-version=1 --all-features --no-deps)

QUERY='
# store the input for multiple passes
. as $input |

# first pass: collect the name and version for each local package
$input.packages[] | [{ key: .name, value: .version }] |
from_entries as $versions |

# second pass: for each package, emit a single object for each (package,
# dependency) pair where the dependency is also a local package
$input.packages[] | {
    name,
    version,
    dependency: (.dependencies[] | {
        name,
        version: $versions[.name]
    } | select(.version != null))
} |

# join the dependency and package with a tab, then sort topologically
"\(.dependency.name)/v\(.dependency.version)\t\(.name)/v\(.version)"
'

jq -r "${QUERY}" <<< "$METADATA" | tsort
