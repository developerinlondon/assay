# Migrating to assay 0.14.1 / assay-workflow 0.3.1 / assay-engine 0.2.1

Patch release. **No breaking changes.** Read this only if you fall into one of the cases below.

## You're running an `assay-engine` binary

Redeploy from the new release to pick up the `workflow.cancel` empty-body fix (#66). Without the new
binary, callers that rely on `assay.workflow.cancel` (or any `workflow.client():cancel(id)`) on the
wire still see 400 errors.

## You consume `assay-workflow` as a library

`cargo update -p assay-workflow` picks up `0.3.1`. The change is internal to the `cancel_workflow`
handler — request/response contract is wider than before (more body shapes accepted, same outputs).
No source changes required.

## You write Lua scripts against the assay binary

New stdlib modules are available — none replace anything that already existed:

| Module            | Use it for                                                           |
| ----------------- | -------------------------------------------------------------------- |
| `assay.ansi`      | Converting / stripping ANSI escapes in log output                    |
| `assay.url`       | Percent-encoding URLs and `application/x-www-form-urlencoded` bodies |
| `assay.tailscale` | Tailscale OAuth2 + REST API (auth keys, devices, ACLs)               |
| `assay.version`   | Comparing versions across semver / debian / rpm / numeric            |
| `assay.compress`  | gunzip / unxz / unzstd of HTTP bodies and files                      |
| `assay.apt`       | Reading `Packages` indexes from apt repositories                     |

`assay.github` gained module-level helpers for GitHub Releases (`github.latest_release`,
`github.find_asset`, `github.release_checksum`, etc.) — the existing `github.client(...)` API is
unchanged.

`template.render_with_loader(dir, name, vars)` is new — use it when your template wants to
`{% extends %}`, `{% include %}`, or `{% import %}` a sibling template. The existing
`template.render` / `template.render_string` still work for self-contained templates.

## You workaround `workflow.cancel` with a raw HTTP call

You can drop the workaround. Both layers are fixed:

- The stdlib `assay.workflow:cancel(id)` now sends no body and no `Content-Type` header (was sending
  `Content-Type: application/json` with body `[]`).
- The server-side handler accepts no body, `{}`, `[]`, or `{"reason":"..."}` — anything reasonable
  now succeeds.

## You hit issue #40 (Lua coroutine resume nil-to-function)

That bug was filed against `assay 0.10.4` and the long-removed `temporal.worker(...)` API. The
v0.13.0 engine rewrite replaced the mlua `create_thread` path with pure-Lua `coroutine.create` (see
`stdlib/engine/workflow/worker.lua`), which inherits globals from the parent state. The 0.14.1
release adds a regression test (`crates/assay/tests/coroutine_ctx_resume.rs`) pinning this contract
— no script-side change required.

## You're holding off on #75 (OpenSSL drop)

#75 is intentionally not in this patch. It's tracked for a later minor because it touches the
`assay-auth` crate and changes a build-time dependency tree (replacing OpenSSL inside `webauthn-rs`
with RustCrypto).
