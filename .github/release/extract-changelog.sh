#!/usr/bin/env bash
#
# Usage: extract-changelog.sh <version>
#
# Prints the markdown body of the CHANGELOG.md section for the given version.
# Reads CHANGELOG.md from the repo root (current working directory when invoked
# by GitHub Actions). The section runs from the `## [<version>]` heading up to
# (but not including) the next `## [` heading.
#
# Exits 0 on success, 1 if the version isn't found in the changelog.

set -euo pipefail

version="${1:?usage: extract-changelog.sh <version>}"

changelog="CHANGELOG.md"
if [ ! -f "$changelog" ]; then
    echo "error: $changelog not found (run from repo root)" >&2
    exit 1
fi

# `## [x.y.z]` is the section anchor. String-prefix matching (no regex) sidesteps
# awk's escape-sequence warnings and still safely distinguishes `0.1.0` from
# `0.1.10` because we check both the prefix AND the character immediately after
# the version (which must be `]`).
anchor="## [${version}]"

body=$(awk -v anchor="$anchor" '
    index($0, anchor) == 1 { in_section = 1; next }
    in_section && index($0, "## [") == 1 { exit }
    in_section { print }
' "$changelog")

if [ -z "$body" ]; then
    echo "error: no section found for version ${version} in ${changelog}" >&2
    exit 1
fi

# Strip leading/trailing blank lines for cleanliness.
printf '%s\n' "$body" | awk '
    NF { first = first ? first : NR; last = NR; lines[NR] = $0 }
    !NF { lines[NR] = $0 }
    END {
        for (i = first; i <= last; i++) print lines[i]
    }
'
