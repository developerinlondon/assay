---
category: Cloud & AWS
---

## assay.aws.ecr

AWS Elastic Container Registry client. Get authorization tokens for `docker login` or
pushing/pulling images. Uses `assay.aws.sigv4` internally for request signing.

### Client

- `ecr.client(opts)` → client. `opts` is a table with the fields:
  - `access_key` _(required)_ — AWS access key ID
  - `secret_key` _(required)_ — AWS secret access key
  - `region` _(required)_ — AWS region
  - `session_token` _(optional)_ — STS session token
  - `endpoint` _(optional)_ — override the API endpoint (full URL or bare host). Defaults to
    `https://api.ecr.<region>.amazonaws.com`. Useful for VPC endpoints or for injecting a mock
    server in tests.

### Authorization

- `c:get_authorization_token()` → `{token, proxy_endpoint, expires_at}` — Get an ECR authorization
  token. The `token` field is the password for `docker login -u AWS`.

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
