---
category: Cloud & AWS
tagline: AWS credential resolution + STS, in-process — the credential chain, what each path sends, and why readonly admits it
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

STS speaks the Query protocol over `POST`, so half the credential chain sends a request readonly
would once have refused on its verb alone. It no longer does. Readonly and approval both read an AWS
request for what it asks for — the service out of the SigV4 credential scope, the action out of
`X-Amz-Target`, the form body or the query string — and `AssumeRole`, `AssumeRoleWithWebIdentity`
and `GetCallerIdentity` only trade one credential for another. So every row below works with no
policy file:

| How credentials are obtained                          | Request to STS                   | Readonly, no policy |
| ----------------------------------------------------- | -------------------------------- | ------------------- |
| `opts.access_key` + `opts.secret_key`                 | none                             | works               |
| `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY`         | none                             | works               |
| profile with static keys and no `role_arn`            | none                             | works               |
| profile with static keys **and** `role_arn`           | `AssumeRole` POST                | works               |
| profile with `role_arn` + `source_profile`            | `AssumeRole` POST                | works               |
| profile with `web_identity_token_file`                | `AssumeRoleWithWebIdentity` POST | works               |
| IRSA (`AWS_ROLE_ARN` + `AWS_WEB_IDENTITY_TOKEN_FILE`) | `AssumeRoleWithWebIdentity` POST | works               |
| any of the above plus `opts.role_arn`                 | `AssumeRole` POST                | works               |

Readonly plus IRSA simply works, which is the point: nobody running a health check in a pod should
have to write a policy rule to let the pod authenticate.

What the exemption is not is a blanket pass for AWS. It is an enumerated list of read actions, so
anything else is refused whatever verb carries it:

```
readonly: http.get blocked (aws ec2 Action=TerminateInstances is not a read)
```

That closes a hole the verb rule left open — the Query API takes its action in the query string, so
a signed `GET ?Action=TerminateInstances` used to pass. Two STS actions are deliberately off the
list: `GetSessionToken` and `GetFederationToken` mint credentials for someone else to use, which is
not a read however their names read.

The host has to be AWS's own — `amazonaws.com`, `amazonaws.com.cn` or `api.aws`, matched on label
boundaries, so `evilamazonaws.com` is not one. And the host has to back the service the signature
claims: the labels from the service's own to the suffix are AWS's to assign, so a name ending
`sts.us-east-1.amazonaws.com` really is STS, while `abc.execute-api.us-east-1.amazonaws.com` is a
customer's own API Gateway code and `sts.s3.us-east-1.amazonaws.com` is a bucket called `sts`. A
signature is a claim, not a capability: anyone can write an `Authorization` header naming the `sts`
service, and anywhere but STS's own hosts it buys nothing — the request falls back to its verb, so a
`POST` stays a write.

On a host an AWS API service owns (EC2, IAM, STS and the other Query-protocol services, ECR, S3) the
action decides whether or not the request is signed, and authentication the classifier cannot read
is a write, never "unsigned": a SigV4 `Authorization` is read by its `Credential`, `SignedHeaders`
and `Signature` components with the algorithm in any case, and SigV2 query signing is not read at
all. A readable scope naming a service the host does not serve is a write. So is AWS signing
material sent to a host that is not an AWS name, such as an IP literal or a wildcard-DNS alias,
except a readable `s3`-scoped `GET` or `HEAD`, which keeps S3-compatible stores such as MinIO
readable. A caller-set `Host` header that names a different host than the URL is refused, and every
redirect is classified again before it is followed, `http.download`'s included.

A request to an AWS host that is ambiguous about what it asks is refused rather than guessed at: two
`Action` parameters (in any mix of case, query and form body), two `X-Amz-Credential` scopes, or two
header names that differ only in case. `http` sends one value per lowercased header name, so a
second `Authorization` spelled `authorization` would otherwise let the classifier read one value
while AWS received the other.

```
readonly: http.get blocked (aws request names a header twice)
```

Regional, FIPS, dual-stack and interface-VPC-endpoint names all qualify, so an IRSA pod pointed at
`vpce-0123abc.sts.us-east-1.vpce.amazonaws.com` by `sts_endpoint` works like any other.

### When a policy takes over

`http.rules` is default-deny, so a policy that names only its own endpoints refuses the AWS calls
the built-in would have admitted, and has to admit their hosts itself. A rule without `classify`
admits the host and leaves the built-in to say what the request does: the exchange below stays a
read, and a mutating `GET` on the EC2 rule stays a write. An explicit `classify:` overrides the
built-in either way:

```yaml
version: 1
http:
  rules:
    - hosts: ["sts.us-east-1.amazonaws.com"]
      methods: [POST]
      paths: ["/"]
    - hosts: ["ec2.us-east-1.amazonaws.com"]
      methods: [GET]
      paths: ["/"] # Describe* reads; TerminateInstances is still refused
```

Name the regional endpoint the script actually uses (`sts.<region>.amazonaws.com`, from `region` /
`AWS_REGION`), not a wildcard. A literal host is an exact match, so a rule naming `us-east-1` does
nothing for a client built with `region = "eu-west-1"` — write out the region you mean rather than
reaching for `*.amazonaws.com`, which would cover every region and every AWS service at once. STS
serves `/` only, so scoping the path costs nothing. If a rule does need `classify: read`, keep it as
narrow as this: [the policy reference](../policy.md) warns that a broad one hands back the write
gate.

### Downstream modules

`assay.aws.ec2` and `assay.aws.s3` issue only signed `GET` requests, and `assay.aws.ecr`'s
`GetAuthorizationToken` is a signed `POST` that mints a short-lived registry password; all three are
reads, as is whichever credential hop the chain takes to reach them. `assay.aws.eks`'s token minting
is pure presigning and issues no request of its own.

Example — a readonly health check on an IRSA-authenticated pod, no policy file:

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
