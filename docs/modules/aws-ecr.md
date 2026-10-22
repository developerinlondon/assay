---
category: Cloud & AWS
---

## assay.aws.ecr

AWS Elastic Container Registry client. Get authorization tokens for `docker login` or
pushing/pulling images. Uses `assay.aws.sigv4` internally for request signing.

### Client

- `ecr.client(opts)` → client. `opts` is a table with the fields:
  - `access_key` _(optional)_ — AWS access key ID. Omit it and credentials are resolved through
    `assay.aws.sts` (profile / env / IRSA); pass it together with `secret_key`.
  - `secret_key` _(optional)_ — AWS secret access key
  - `region` _(required)_ — AWS region (or `AWS_REGION` / `AWS_DEFAULT_REGION`)
  - `profile` / `role_arn` _(optional)_ — passed through to `assay.aws.sts` when resolving
  - `session_token` _(optional)_ — STS session token
  - `endpoint` _(optional)_ — override the API endpoint (full URL or bare host). Defaults to
    `https://api.ecr.<region>.amazonaws.com`. Useful for VPC endpoints or for injecting a mock
    server in tests.

### Authorization

- `c:get_authorization_token()` → `{token, proxy_endpoint, expires_at}` — Get an ECR authorization
  token. The `token` field is the password for `docker login -u AWS`.

### Readonly and approval modes

`GetAuthorizationToken` is a read that travels as a write: ECR's JSON protocol sends it as a signed
`POST /` with an `x-amz-json-1.1` body, and readonly mode classifies by verb. So the call fails
under readonly with `readonly: http.post blocked`, and suspends for a human decision under approval
— even though it only mints a short-lived registry password and changes nothing.

Two gated hops, not one: resolving credentials can also `POST` to STS when they come from IRSA or an
assumed role (see [assay.aws.sts](aws-sts.md)). A policy that wants the whole flow readonly has to
declare both endpoints reads:

```yaml
version: 1
http:
  rules:
    - hosts: ["sts.us-east-1.amazonaws.com"]
      methods: [POST]
      paths: ["/"]
      classify: read # credential exchange — a read
    - hosts: ["api.ecr.us-east-1.amazonaws.com"]
      methods: [POST]
      paths: ["/"]
      classify: read # GetAuthorizationToken — a read
```

Name the regional hosts the script actually uses rather than a wildcard. A literal host is an exact
match, so a rule for one region does nothing for a client built in another — write out the region
rather than reaching for `*.amazonaws.com`, which would cover every region and every AWS service.
Both endpoints serve `/` only, so pinning the path costs nothing and keeps the rule off the rest of
the account.

Those two rules cover an ECR-only script and nothing more. `http.rules` is default-deny, so if the
same script also reads EC2 or S3, those endpoints need rules of their own or they are refused with
`policy: GET … is not allowed by any http rule`.

Note what the second rule does and does not buy: it admits the token mint, not the push. Writing an
image to the registry goes through `oci.copy`, which readonly refuses outright — it is a blocked
function, not an HTTP call policy can reclassify. So the end-to-end push below is not a readonly
flow, and no policy makes it one.

Example:

```lua
local ecr = require("assay.aws.ecr")
local c = ecr.client({
  access_key    = env.get("AWS_ACCESS_KEY_ID"),
  secret_key    = env.get("AWS_SECRET_ACCESS_KEY"),
  session_token = env.get("AWS_SESSION_TOKEN"),
  region        = "us-east-1",
})
local auth = c:get_authorization_token()
print(auth.token)           -- Password for docker login
print(auth.proxy_endpoint)  -- Registry URL
print(auth.expires_at)      -- Token expiry
```

### End-to-end: push an image to ECR

```lua
local ecr = require("assay.aws.ecr")
local c = ecr.client({
  access_key    = env.get("AWS_ACCESS_KEY_ID"),
  secret_key    = env.get("AWS_SECRET_ACCESS_KEY"),
  session_token = env.get("AWS_SESSION_TOKEN"),
  region        = "us-east-1",
})
local auth = c:get_authorization_token()

oci.copy(
  "registry.gitlab.com/team/app:abc123",
  "111122223333.dkr.ecr.us-east-1.amazonaws.com/app:abc123",
  {
    src_auth = { username = env.get("CI_REGISTRY_USER"), password = env.get("CI_REGISTRY_PASSWORD") },
    dst_auth = { username = "AWS", password = auth.token },
  }
)
```
