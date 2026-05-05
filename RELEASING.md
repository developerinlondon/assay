# Releasing

Two independent tracks. The binary release and library releases share a repo but no longer share a
release cycle.

```text
   push to main                          workflows                          GitHub releases
══════════════════════           ══════════════════════           ═══════════════════════════════

crates/assay/Cargo.toml ──bump──►   release.yml             ──►   assay-lua-v<X.Y.Z>
                                    (assay binary)                 ├── assay-linux-x86_64
                                                                   ├── assay-darwin-aarch64
                                                                   └── lua-checksums.txt
                                                                              ▲
                                                                              │ GET binary
                                                                              │
libs/<name>/VERSION     ──bump──►   release-libs.yml        ──►   assay-lib-<name>-v<libver>
                                    (per-lib tarball)              ├── assay-lib-<name>-<libver>.tar.gz
                                                                   └── assay-lib-<name>-<libver>.tar.gz.sha256
                                                                              ▲
                                                                              │ GET per-lib URL
                                                                              │
                                                                   ┌──────────┴──────────┐
                                                                   │ assay install       │
                                                                   │   (client)          │
                                                                   └─────────────────────┘
```

## Releasing the binary

1. Bump `crates/assay/Cargo.toml` `version` (and `Cargo.lock` to match).
2. Add a `## assay-lua <X.Y.Z> — <date>` section to `CHANGELOG.md`.
3. Open a PR titled `release: assay-lua <X.Y.Z>`. Squash-merge.
4. `release.yml` fires on push to `main`, sees the new version, builds the Linux + macOS binaries,
   tags `assay-lua-v<X.Y.Z>`, creates the release.

The release-existence check is by tag — re-running the workflow on the same version is a no-op.

## Releasing a library

1. Bump `libs/<name>/VERSION`.
2. Add a `## <name> <X.Y.Z> — <date>` section to `CHANGELOG.md` (optional — the release notes fall
   back to a generated stub if missing).
3. Open a PR. Squash-merge.
4. `release-libs.yml` fires on push to `main` when any `libs/*/VERSION` changes, builds a flat
   tarball of `libs/<name>/` (excluding tests), tags `assay-lib-<name>-v<libver>`, creates the
   release.

Idempotent: already-released versions are skipped. Manual re-run via `workflow_dispatch` is safe.

## Consumer install

`assay install` reads a consumer's `Manifest.lua` and resolves each lib to its per-lib release URL
(default `…/releases/download/assay-lib-<name>-v<libver>/assay-lib-<name>-<libver>.tar.gz`),
downloads, verifies sha256, extracts into `<lib_dir>/<name>/`. See
[`docs/modules/install.md`](docs/modules/install.md) for the consumer side.

## Design notes

Architecture rationale and the install protocol live in
[`.claude/plans/21-libs-folder-and-install.md`](.claude/plans/21-libs-folder-and-install.md).
