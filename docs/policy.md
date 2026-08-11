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

## What it does not do

- It does not revalidate redirects. Set `follow_redirects = false` on the client for now if that
  matters.
- It does not restrict filesystem reads.
- It does not resolve credentials; a script that needs a secret still reads it from an allowlisted
  environment key.
