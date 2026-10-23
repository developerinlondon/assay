#!/bin/sh
# This repository has one author, its owner. A commit crediting anyone or
# anything else fails, including every Co-authored-by line whoever it names,
# and so does one authored or committed by an admin account.
# POSIX sh: the mirror job runs it in alpine/git, which has no bash.
# usage: scripts/check-commit-credits.sh <base>..<head>
set -eu

range=${1:?usage: $0 <base>..<head>}
tools='claude|copilot|cursor|codex|chatgpt|gpt|opencode|sisyphus'
pattern="co-authored-by:|assisted-by:|claude-session:|generated (with|by)[[:space:]]+\[?($tools)|ultraworked with|noreply@anthropic\.com"

commits=$(git rev-list "$range")
if [ -z "$commits" ]; then
    echo "check-commit-credits: no commits in $range" >&2
    exit 1
fi

failed=0
for c in $commits; do
    subject=$(git log -1 --format='%h %s' "$c")
    if hits=$(git log -1 --format=%B "$c" | grep -Ei "$pattern"); then
        echo "check-commit-credits: $subject has a credit line:" >&2
        printf '    %s\n' "$hits" >&2
        failed=1
    fi
    if git log -1 --format='%an%n%cn' "$c" | grep -Eqix 'admin(istrator)?'; then
        echo "check-commit-credits: $subject is authored or committed by an admin account" >&2
        failed=1
    fi
    if git log -1 --format='%ae%n%ce' "$c" | grep -Eqi '^admin@'; then
        echo "check-commit-credits: $subject has an admin@ author or committer email" >&2
        failed=1
    fi
done

[ "$failed" -eq 0 ] || exit 1
echo "check-commit-credits: $(echo "$commits" | wc -l | tr -d ' ') commit(s) in $range are clean"
