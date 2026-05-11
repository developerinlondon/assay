---
category: Cloud & AWS
---

## assay.aws.ecr

AWS Elastic Container Registry client. Get authorization tokens for `docker login` or pushing/pulling
images. Uses `assay.aws.sigv4` internally for request signing.

### Client

- `ecr.client(access_key, secret_key, session_token, region)` → client

### Authorization

- `c:get_authorization_token()` → `{token, proxy_endpoint, expires_at}` — Get an ECR
  authorization token. The `token` field is the password for `docker login -u AWS`.

Example:

```lua
local ecr = require("assay.aws.ecr")
local c = ecr.client(
  "AKIAIOSFODNN7EXAMPLE",
  "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
  "",  -- session_token
  "us-east-1"
)
local auth = c:get_authorization_token()
print(auth.token)           -- Password for docker login
print(auth.proxy_endpoint)  -- Registry URL
print(auth.expires_at)      -- Token expiry
```
