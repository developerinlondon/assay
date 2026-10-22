---
category: Cloud & AWS
tagline: AWS credential resolution + STS, in-process — the credential chain, which paths POST, and what that costs in readonly mode
---

## assay.aws.sts

Resolves AWS credentials from the same sources the AWS CLI reads — with one ordering difference,
noted below — and assumes roles, entirely in-process, with no `aws` subprocess. Every other
`assay.aws.*` module calls into it: build a client without explicit `access_key`/`secret_key` and it
resolves them through this chain.

### Credential resolution

`sts.credentials(opts?)` → `{access_key, secret_key, session_token?}`, trying in order:

1. `opts.access_key` + `opts.secret_key` — explicit, used verbatim.
2. `opts.profile` (or `AWS_PROFILE`) — `~/.aws/credentials` and `~/.aws/config`, following
   `role_arn` + `source_profile` chains and `web_identity_token_file`.
3. `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` (+ `AWS_SESSION_TOKEN`).
4. IRSA: `AWS_ROLE_ARN` + `AWS_WEB_IDENTITY_TOKEN_FILE`, as mounted into an in-cluster pod.

One deviation from the AWS CLI worth knowing: a profile is consulted _before_ the environment keys,
where the CLI lets `AWS_ACCESS_KEY_ID` win. So with `AWS_PROFILE` set, steps 3 and 4 are never
reached and the profile decides — including whether an STS POST happens.

If `opts.role_arn` is set, whatever those steps resolved then assumes that role. Temporary
credentials are cached in-process until shortly before they expire, so a script that builds several
clients pays for the exchange once.

`opts` also takes `region` and `sts_endpoint`, plus `config_file` / `credentials_file` to point the
profile lookup at files other than `~/.aws/*`.

`session_name` and `duration_secs` are not reliably plumbed through `credentials`: resolving a
profile names the session itself (`assay-<profile>`), and applying `opts.role_arn` forwards neither,
so both fall back to `assay` and one hour. Set them by calling `assume_role` or
`assume_role_with_web_identity` directly.

### Functions

- `sts.credentials(opts?)` → `creds` — the chain above.
- `sts.assume_role(role_arn, opts?)` → `creds` — `sts:AssumeRole`, signed with credentials resolved
  from the chain (or `opts.credentials`). Honors `session_name` and `duration_secs`.
- `sts.assume_role_with_web_identity(role_arn, token, opts?)` → `creds` —
  `sts:AssumeRoleWithWebIdentity`. Unsigned: the OIDC token itself authenticates the call. Honors
  `session_name` and `duration_secs`.

### Readonly and approval modes

STS speaks the Query protocol over `POST`, and readonly mode classifies by verb — so which half of
the chain you land in decides whether a readonly script can get credentials at all:

| How credentials are obtained                          | Request to STS                   | Readonly, no policy |
| ----------------------------------------------------- | -------------------------------- | ------------------- |
| `opts.access_key` + `opts.secret_key`                 | none                             | works               |
| `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY`         | none                             | works               |
| profile with static keys and no `role_arn`            | none                             | works               |
| profile with static keys **and** `role_arn`           | `AssumeRole` POST                | **blocked**         |
| profile with `role_arn` + `source_profile`            | `AssumeRole` POST                | **blocked**         |
| profile with `web_identity_token_file`                | `AssumeRoleWithWebIdentity` POST | **blocked**         |
| IRSA (`AWS_ROLE_ARN` + `AWS_WEB_IDENTITY_TOKEN_FILE`) | `AssumeRoleWithWebIdentity` POST | **blocked**         |
| any of the above plus `opts.role_arn`                 | `AssumeRole` POST                | **blocked**         |

A blocked exchange fails with `readonly: http.post blocked`, raised from inside whichever client
asked for credentials — so an `assay.aws.ec2` script whose own requests are all `GET` can still fail
this way before it issues one.

Exchanging a token for credentials has no side effect, which is what `classify: read` is for.
Declare the STS endpoint a read and the chain runs under readonly, and stops suspending under
approval:

```yaml
version: 1
http:
  rules:
    - hosts: ["sts.us-east-1.amazonaws.com"]
      methods: [POST]
      paths: ["/"]
      classify: read # credential exchange — a read
```

Name the regional endpoint the script actually uses (`sts.<region>.amazonaws.com`, from `region` /
`AWS_REGION`), not a wildcard. A literal host is an exact match, so a rule naming `us-east-1` does
nothing for a client built with `region = "eu-west-1"` — write out the region you mean rather than
reaching for `*.amazonaws.com`, which would cover every region and every AWS service at once. STS
serves `/` only, so scoping the path costs nothing and keeps the rule from being the broad
`classify: read` that [the policy reference](../policy.md) warns hands back the write gate. The
exemption covers exactly the host, method and path it names — the ECR and EC2 endpoints are
untouched by it.

What that rule does not do is leave the rest of the script alone. `http.rules` is default-deny: once
the list exists, any request matching no rule is refused before the socket is opened, `GET`
included.

```
policy: GET ec2.us-east-1.amazonaws.com/ is not allowed by any http rule
```

So a policy naming only STS breaks the very reads it was added to enable. Every endpoint the script
touches needs a rule. Read rules for `GET` need no `classify` — the verb already defaults to a read.

Without a policy file, nothing changes — readonly keeps blocking the exchange. Approval mode needs
no rule to work; it suspends the POST for a human decision instead of refusing it.

### Downstream modules

`assay.aws.ec2` and `assay.aws.s3` issue only signed `GET` requests, so they run under readonly
untouched whenever the chain lands on one of the "works" rows above — explicit keys, env keys with
no `AWS_PROFILE` set, or a static-keys profile without a `role_arn`. `assay.aws.ecr`'s
`GetAuthorizationToken` is itself a signed `POST` and needs its own read rule on
`api.ecr.<region>.amazonaws.com`; see [assay.aws.ecr](aws-ecr.md). `assay.aws.eks`'s token minting
is pure presigning and issues no request of its own.

Example — a readonly health check on an IRSA-authenticated pod. Two hops, so two rules, and the
region has to be the same one the script asks for:

```yaml
version: 1
http:
  rules:
    - hosts: ["sts.us-east-1.amazonaws.com"]
      methods: [POST]
      paths: ["/"]
      classify: read # IRSA credential exchange — a read
    - hosts: ["ec2.us-east-1.amazonaws.com"]
      methods: [GET]
      paths: ["/"] # Describe* reads — GET needs no classify
```

```lua
local ec2 = require("assay.aws.ec2")
-- No keys: credentials come from AWS_ROLE_ARN + AWS_WEB_IDENTITY_TOKEN_FILE
-- via assay.aws.sts, which POSTs to STS once and caches the result.
local c = ec2.client({ region = "us-east-1" })

for _, i in ipairs(c:describe_instances({
  filters = { { name = "instance-state-name", values = { "running" } } },
})) do
  print(i.instance_id, i.instance_type, i.availability_zone)
end
```
