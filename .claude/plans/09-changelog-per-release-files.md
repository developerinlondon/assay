# Plan 09: CHANGELOG split into per-release files

## Problem

`CHANGELOG.md` is a single file, grown to ~1400 lines across the v0.10.x
and v0.11.x series. Two real costs:

1. **AI-agent context window.** When an agent needs to reason about
   what changed in a specific release — triaging a bug report,
   reviewing a consumer's dependency update, answering a
   "when did feature X ship" question — it has to load the whole file
   just to find one section. That's tens of thousands of tokens for
   what's usually a scoped question.

2. **Git diff noise.** Every release pushes a ~40-line addition to the
   top of a long file. Reviewers see a "+40 / -0" on a 1400-line file,
   which reviews fine in isolation but grows harder to scan across
   releases as the file balloons.

Additional irritations:

- Humans skimming the changelog for "what's new in 0.11.10" scroll
  past everything after it.
- The site's `changelog.html` renders the whole file as one tall page,
  similarly hard to navigate.

## Goal

Split the changelog into per-release files under `changelog/`, one
file per version, with a thin auto-generated `CHANGELOG.md` at the repo
root that concatenates the per-version files in descending-version
order for backward compatibility with tools that expect the root file.
Update the authoring flow and release-docs checklist so developers
(and agents) only ever edit a single per-release file.

## Non-goals

- Changing the changelog *content*. The split is pure file
  reorganisation; the words don't move or change meaning.
- Replacing the changelog with GitHub Releases as the source of truth.
  That was considered and rejected — GitHub releases are a good
  secondary surface but the in-tree file is what downstream consumers
  and offline readers rely on.
- Dropping the auto-generated root `CHANGELOG.md`. Some tools (badge
  renderers, changelog parsers, IDE plugins) expect a file at that
  exact path. Generating it is trivial; removing it breaks ecosystem
  integrations for a marginal win.

## Design

### Directory layout

```
assay/
├── changelog/
│   ├── v0.11.13.md        ← one file per release, edited by hand
│   ├── v0.11.12.md
│   ├── v0.11.11.md
│   ├── ...
│   └── v0.1.0.md
├── CHANGELOG.md           ← auto-generated from changelog/*.md
└── .github/workflows/
    └── changelog.yml      ← CI job that regenerates CHANGELOG.md
```

### Per-version file format

Each file is the existing `## [0.x.y] - YYYY-MM-DD` entry, verbatim,
but without the `## [...]` heading (the filename carries the version).

```md
# v0.11.13 — 2026-04-17

## Changed

- Full workflow IDs in the detail view. …
- Smart truncate. …

## Tests

- 32 lib + 40 orchestration tests pass.
```

The filename is the canonical version identifier. The top `#` heading
matches the filename and adds a date.

### Root CHANGELOG.md

Generated from `changelog/*.md` in descending-semver order:

```md
# Changelog

All notable changes to Assay are documented here.
Per-release entries live in changelog/; this file is auto-generated.

<!-- v0.11.13 -->
## [0.11.13] - 2026-04-17

### Changed
...

<!-- v0.11.12 -->
## [0.11.12] - 2026-04-17

### Added
...
```

The generator normalises the per-version heading (`# v0.11.13 —
2026-04-17`) into the Keep-a-Changelog standard heading
(`## [0.11.13] - 2026-04-17`) and stitches in the version's body. Comment
markers bracket each version for easy in-file navigation.

### Generation mechanism

Small shell or Lua script at `scripts/build-changelog.sh` (or
`build-changelog.lua` — consistent with the rest of assay's tooling):

```sh
#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
{
  echo "# Changelog"
  echo
  echo "All notable changes to Assay are documented here."
  echo "Per-release entries live in \`changelog/\`; this file is auto-generated."
  echo
  for f in $(ls "$ROOT/changelog"/v*.md | sort -V -r); do
    version="$(basename "$f" .md)"
    echo "<!-- $version -->"
    # Transform "# v0.11.13 — 2026-04-17" → "## [0.11.13] - 2026-04-17"
    sed -E "1 s/^# v([0-9.]+) — ([0-9-]+)$/## [\\1] - \\2/" "$f"
    echo
  done
} > "$ROOT/CHANGELOG.md"
```

Run by a CI job on every push to `main`, or as a pre-commit / release
step. Idempotent — running it when nothing changed produces the same
output.

### Release flow (updated AGENTS.md checklist)

Before → after on the "Non-source files" entry in AGENTS.md:

**Before:**

> - `CHANGELOG.md` — new section at the top; describe the
>   OIDC/Kubernetes/HTTP scenario enabled, not a specific consumer.

**After:**

> - `changelog/v{version}.md` — **new** file per release. Describe the
>   OIDC/Kubernetes/HTTP scenario enabled, not a specific consumer.
>   `CHANGELOG.md` is auto-generated from these; do **not** edit it
>   directly — the CI regeneration step will overwrite hand edits.

Pre-push verification gains one line:

```sh
# Ensure the auto-generated CHANGELOG.md is in sync with the per-release files
scripts/build-changelog.sh && git diff --exit-code CHANGELOG.md
```

### Site rendering

`site/pages/changelog.html` currently renders the whole file as one
tall page. Update to a two-pane layout:

```
┌─ Versions ───┬─ v0.11.13 — 2026-04-17 ────────────────────┐
│ v0.11.13 (▸) │                                            │
│ v0.11.12     │ ### Changed                                │
│ v0.11.11     │ - Full workflow IDs in the detail view …   │
│ v0.11.10     │ - Smart truncate …                         │
│ …            │ …                                          │
└──────────────┴────────────────────────────────────────────┘
```

- Left: scrollable version list, newest first.
- Right: the currently-selected version's body, rendered from its
  `changelog/v*.md` file.
- URL hash reflects selection (`/changelog#v0.11.10`) so versions can
  be linked directly.

Site build (`site/build.lua`) reads the folder, emits a JSON index
(`[{version, date, excerpt}, ...]`) and per-version HTML fragments;
client-side JS swaps fragments on version selection.

## Migration

One-time script to carve the existing `CHANGELOG.md` at `## [x.y.z]`
boundaries into per-version files. ~20 lines of shell using `awk` to
split on heading markers, rename the heading to the per-file format,
write each to `changelog/v{version}.md`. Review the split output
(`ls changelog/` and diff against the original concatenated) to confirm
byte-accuracy before removing the hand-written `CHANGELOG.md`.

## Rollout

One PR, one merge. Suggested sequence:

1. Run the migration script, commit the per-version files and the
   generator script.
2. Run the generator; commit the regenerated `CHANGELOG.md`. Diff
   against the pre-migration `CHANGELOG.md` should be empty (or only
   whitespace) — that's the "split is faithful" check.
3. Add the CI job to `.github/workflows/ci.yml` that re-runs the
   generator on every push and fails if `CHANGELOG.md` drifts from
   its per-file sources.
4. Update `AGENTS.md` release-docs checklist.
5. (Follow-up PR, not blocking) Update `site/pages/changelog.html` +
   `site/build.lua` for the two-pane render.

## Estimate

30–60 minutes for steps 1–4 (migration + CI wiring). Another
30–60 minutes for step 5 (site rendering), can ship separately.

## Open questions

- [ ] Shell vs Lua for the generator. Shell is ubiquitous but less
      readable for the regex transform; assay's own scripts are Lua
      increasingly, so maybe consistent. Pick whichever lands first.
- [ ] Where to source the date in the per-version heading — the git
      commit date of the tag? `date` at release time? Leave hand-
      authored (current behaviour)? Leaning toward hand-authored for
      explicitness and because the release checklist already asks the
      human to set it.
- [ ] Migration preserves existing `## Fixed / ## Added / ##  Changed`
      sub-headings as `### Fixed / etc` (one level deeper) since the
      per-file `#` heading is the top level. Verify this maps cleanly
      on every release.

## References

- Existing `CHANGELOG.md` — gets split.
- `AGENTS.md` "Release docs checklist" — gets a small wording update.
- `site/pages/changelog.html` + `site/build.lua` — site-render follow-up.
- Keep-a-Changelog format — root `CHANGELOG.md` stays compliant.
