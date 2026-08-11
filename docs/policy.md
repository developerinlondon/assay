# Capability policy

A policy file narrows what a script may reach: which modules it can `require`, which environment
keys it can read, and which HTTP targets it can call. It is enforced inside the runtime, so a script
cannot talk its way around it the way it can around a caller that inspects source text before
running it.

Policy is **orthogonal to execution mode**. The mode (`readonly`, `approval`, unrestricted) decides
whether a _mutating_ operation runs, suspends, or is refused. The policy decides what is _reachable_
at all, in every mode. Load both when you want a script confined to a target list and gated on
writes.

With no policy loaded, every check passes and behaviour is exactly as before.

## Loading

Set `ASSAY_POLICY_FILE` to a path, and every VM the process creates is policed:

```sh
ASSAY_POLICY_FILE=/etc/assay/policy.yaml assay run inventory.lua
```

Embedders can pass one directly instead:

```rust
let policy = Arc::new(assay::lua::policy::Policy::load("/etc/assay/policy.yaml")?);
let lua = assay::lua::create_vm_with_policy(client, options, Some(policy))?;
```

A malformed file is a hard error at VM creation — it never degrades to "unpoliced".

## File format

```yaml
version: 1

modules:
  allow: [assay.openstack, assay.json]

env:
  allow: [OS_PROJECT_NAME]

http:
  max_response_bytes: 262144
  redact: [password, token, secret, authorization, x-auth-token]
  rules:
    - hosts: ["*.identity.example.com"]
      methods: [GET]
      paths: ["/v3/*"]

    - hosts: ["*.identity.example.com"]
      methods: [POST]
      paths: ["/v3/auth/tokens"]
      classify: read
```

Unknown keys are rejected rather than ignored — a typo in an allowlist that silently widened the
policy is the worst possible failure. Every section is optional, and an absent section means
unrestricted, so adding a section can only ever tighten a file.

### `modules.allow`

An exact list of `assay.*` module names. A `require` outside the list raises
`policy: module '<name>' is not in the allowed set`. Absent means every module is available.

### `env.allow`

An exact list of environment keys. `env.get` returns `nil` for anything else and `env.list` omits it
— a key outside the list is indistinguishable from one that is not set, because presence is itself
information. `allow: []` hides the entire environment. Absent means the whole environment is
readable.

### `http.rules`

An ordered list; the first matching rule wins. When `rules` is present the default is deny, so a
request matching nothing is refused with
`policy: <METHOD> <host><path> is not allowed by any http rule` before the socket is opened.

| Field      | Meaning                                                                                                                                        |
| ---------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| `hosts`    | Required. `*` matches any host; `*.example.com` matches strict subdomains but not the apex; anything else is an exact, case-insensitive match. |
| `methods`  | Empty or absent matches every method.                                                                                                          |
| `paths`    | Empty or absent matches every path. `*` matches within one segment, `**` across segments. Patterns are anchored at both ends.                  |
| `classify` | `read` or `write`. Overrides how the gates treat a match.                                                                                      |

Host patterns deliberately support only a leading `*.` rather than a general glob: `*example.com`
would also match `evilexample.com`, which is a footgun in an allowlist.

### `classify: read`

Some reads authenticate with a POST. OpenStack Keystone issues a token with `POST /v3/auth/tokens`;
minting a Kubernetes bearer token presigns an STS call. Classifying by verb alone marks these as
writes, so `readonly` mode cannot reach the service at all and `approval` mode demands a human
decision for what is only a login.

`classify: read` says "this exact target is a read". A matching request then proceeds under
`readonly`, and does not suspend under `approval`. Everything else on the same host is unaffected:

```yaml
rules:
  - hosts: ["*.identity.example.com"]
    methods: [POST]
    paths: ["/v3/auth/tokens"]
    classify: read # authentication — a read
  - hosts: ["*.compute.example.com"]
    methods: [POST]
    paths: ["/v2.1/servers"]
# creating a server — still a write
```

Scope these narrowly. A `classify: read` rule with a broad path pattern hands back the write gate
you were relying on.

### `http.redact`

Key names stripped from responses before the script sees them. Matching is case-insensitive and
applies to JSON object keys at any depth, plus response header names. Values are replaced with
`[redacted]` rather than removed, so callers that expect a field still find one.

### `http.max_response_bytes`

A response larger than this raises rather than truncating, so a caller cannot mistake a clipped body
for a complete one. The transport buffers the body before the check, so treat this as a disclosure
control — a bound on what reaches the script — not as a memory bound on the process.

## `credentials`

A script that must authenticate has, until now, had to read the secret itself — which means any
script the runtime executes can read it, and send it anywhere the policy allows. Declared
credentials break that link:

```yaml
credentials:
  inventory-ro:
    username: ASSAY_INVENTORY_USER
    password: ASSAY_INVENTORY_PASSWORD
```

Each field names the environment key holding the value. `credential.get("inventory-ro")` returns a
handle whose fields are opaque placeholders, not secrets:

```lua
local c = credential.get("inventory-ro")
local openstack = require("assay.openstack")
local client = openstack.client("https://identity.example.com/v3", {
  username = c.username,
  password = c.password,
  project_name = "demo-project",
})
```

The real values are substituted into the outgoing request body and headers by the HTTP layer, after
the policy has already decided the target is allowed. The script composes an authenticated request
without ever holding the secret: printing, concatenating, or `json.encode`-ing a handle yields the
placeholder. Modules that accept `username`/`password` need no changes.

Pair this with `env.allow` that excludes the backing keys — otherwise the script can simply read the
environment directly and the handle buys nothing.

Requesting a credential the policy does not declare is an error, not an empty handle.

A handle used in a **URL** is refused rather than substituted: a secret in a request line ends up in
every access log along the path.

### Residual risk, stated plainly

Substitution is positional, not semantic. A script can put a handle in a field the target service
does not expect, and the real value will be sent there — to a host the policy already allows. The
`http.rules` allowlist is what bounds this. It is a smaller blast radius than a script that can read
the secret and post it anywhere allowed, but it is not zero, and a policy whose `hosts` list is wide
gives most of that back.

## What it does not do

- It does not revalidate redirects. Set `follow_redirects = false` on the client for now if that
  matters.
- It does not restrict filesystem reads.
- Credential fields resolve from environment keys only. There is no file or secret-manager source
  yet.
