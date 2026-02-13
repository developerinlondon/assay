Assay v0.1.0 — Status Update
All 8 Steps Complete ✅
The last session completed the entire Plan 02 roadmap. Everything is committed on branch feat/v0.1.0 with clean working tree:
| Step | Commit | What |
|------|--------|------|
| 1 | 0405279 | Core builtins: fs.read, crypto.jwt_sign, http.delete, base64, stdlib loader |
| 2 | 8793f8d | Foundation: crypto.hash (SHA2/SHA3), crypto.random, regex |
| 3 | 3312aac | General purpose: fs.write, yaml, toml, async.spawn, direct .lua execution |
| 4 | cc47092 | http.serve() — Lua scripts become web services (hyper 1.x) |
| 5 | b505f9e | Database: db.connect/query/execute (Postgres, MySQL, SQLite via sqlx) |
| 6 | bac2322 | WebSocket client + template engine (tokio-tungstenite, minijinja) |
| 7 | 036cb6f | E2E dogfood tests — Assay testing itself via YAML check mode |
| 8 | 1a3c6fb | Release prep — README, CHANGELOG, version bump, Dockerfile, macOS CI note |
Stats: 490+ tests, 0 failures, 0 clippy warnings, 19 stdlib modules, clean CI.
What Was Proposed Next (your decisions pending)
1. Merge PR — feat/v0.1.0 → main. The branch is ready.
2. Tag v0.1.0 — git tag v0.1.0 && git push --tags
3. GitHub Release — gh release create v0.1.0 --generate-notes
4. crates.io publish — cargo publish (requires crates.io API token setup)
5. macOS CI runner — Added to plan for Step 8 but not yet implemented (GitHub macos-14 Apple Silicon runner)
6. Docker image — Dockerfile exists but no CI publish pipeline yet
So essentially: the code is done, it just needs your go-ahead on merge/tag/publish. Want me to proceed with any of those?


