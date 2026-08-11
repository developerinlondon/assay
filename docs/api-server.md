# HTTP API server

`assay api-serve` exposes gated Lua execution over the network, behind a bearer token:

```sh
ASSAY_API_TOKENS="$(openssl rand -hex 32)" \
ASSAY_POLICY_FILE=/etc/assay/policy.yaml \
  assay api-serve --bind 0.0.0.0:8080
```

`mcp-serve` speaks stdio, which is fine when the client can spawn the runtime as a child process. A
host that wants the runtime in a _separate trust domain_ — its own process, its own credentials,
reachable over the network — has had to invent a protocol over the CLI to get there. This is that
protocol, in the runtime, so every such host does not write its own.

## Endpoints

| Method | Path         | Auth   | Purpose                          |
| ------ | ------------ | ------ | -------------------------------- |
| `GET`  | `/healthz`   | none   | Liveness. Returns `{"ok":true}`. |
| `POST` | `/v1/run`    | bearer | Execute a gated script.          |
| `POST` | `/v1/resume` | bearer | Decide one suspended operation.  |

Both authenticated endpoints return the same tool-mode envelope the CLI prints, with HTTP 200 even
when the run failed — `status` in the body is the outcome (`ok`, `needs_approval`, `error`,
`timeout`). A non-200 means the _request_ was rejected, not the script.

### `POST /v1/run`

```json
{
  "script": "local k8s = require(\"assay.k8s\") return k8s.pods:list(\"default\")",
  "mode": "readonly",
  "args": ["optional", "positional", "args"],
  "timeout_secs": 20
}
```

`mode` is `readonly` or `approval`. `unrestricted` is refused with 400 unless the server was started
with `ASSAY_MCP_UNRESTRICTED=1`, matching `mcp-serve`'s default. `timeout_secs` is clamped to 1–600
rather than trusted.

### `POST /v1/resume`

```json
{ "token": "…", "approve": true, "approver": "someone@example.com" }
```

`approver` is recorded in the resume state and echoed in the envelope. Approval grants are bound to
the exact request that was suspended, so a replay whose target or body changed is refused — see the
approval-mode section of the README.

## Authentication

`ASSAY_API_TOKENS` holds a comma-separated list of accepted bearer tokens. Comparison is
constant-time and does not short-circuit across the list, so response timing does not narrow the
search.

**The server refuses to start when no tokens are configured** — exit code 2. A runtime reachable
over the network with no credential is never what an operator meant, so this fails loudly at boot
rather than quietly serving.

Tokens are equal in power. Per-token policy profiles are not implemented; every run on a given
server is subject to the one `ASSAY_POLICY_FILE` that server was started with. Run separate servers
when callers need different reach.

## Pairing with a policy

The API server is a transport, not a boundary. On its own it will run whatever the mode gate allows,
against whatever the process can reach. Give it a policy:

```yaml
version: 1
modules:
  allow: [assay.openstack]
env:
  allow: [OS_PROJECT_NAME]
credentials:
  inventory-ro:
    username: OS_USERNAME
    password: OS_PASSWORD
http:
  max_response_bytes: 262144
  redact: [password, token]
  rules:
    - hosts: ["*.identity.example.com"]
      methods: [GET]
      paths: ["/v3/*"]
    - hosts: ["*.identity.example.com"]
      methods: [POST]
      paths: ["/v3/auth/tokens"]
      classify: read
```

With that file, a caller can post a script that authenticates and reads inventory, and cannot read
the credential, reach another host, or mutate anything — and because the authentication POST is
declared a read, the whole thing runs under `readonly` with no approval round-trip. See
[`policy.md`](policy.md).

## Operational notes

- **Each run gets its own thread.** The Lua VM is `!Send`, so a run cannot share the server's async
  worker; the server hands each request a dedicated thread and current-thread runtime and takes back
  only the finished envelope. Concurrent runs do not share VM state.
- **Resume state is on local disk** under `ASSAY_STATE_DIR`. A suspended run's resume token is only
  valid against the server instance that issued it, so run a single replica when approval mode is in
  use.
- **Bind to a private interface.** There is no TLS termination here; put it behind whatever your
  platform already uses.
