---
category: Cloud & AWS
---

## assay.aws.sigv4

AWS Signature V4 request signing. Generates `Authorization` headers for authenticated AWS API calls.
Parameterized for any AWS service (ecr, ec2, sts, iam, etc.).

### Signing

- `sigv4.sign(opts)` → headers — Sign an AWS API request. Returns a headers table including
  `authorization`, `x-amz-date`, `x-amz-content-sha256`, and any custom headers passed in. The
  `opts` table:

  | Key             | Type   | Required | Description                             |
  | --------------- | ------ | -------- | --------------------------------------- |
  | `access_key`    | string | Yes      | AWS access key ID                       |
  | `secret_key`    | string | Yes      | AWS secret access key                   |
  | `session_token` | string | No       | AWS session token (for STS credentials) |
  | `service`       | string | Yes      | AWS service name (e.g. `"ecr"`)         |
  | `region`        | string | Yes      | AWS region (e.g. `"us-east-1"`)         |
  | `method`        | string | No       | HTTP method (default `"GET"`)           |
  | `host`          | string | Yes      | API hostname                            |
  | `path`          | string | No       | Request path (default `"/"`)            |
  | `query`         | string | No       | Query string                            |
  | `payload`       | string | No       | Request body (default `""`)             |
  | `headers`       | table  | No       | Additional headers to include and sign  |

Example:

```lua
local sigv4 = require("assay.aws.sigv4")
local headers = sigv4.sign({
  access_key = "AKIA...",
  secret_key = "wJalr...",
  service = "ecr",
  region = "us-east-1",
  method = "POST",
  host = "api.ecr.us-east-1.amazonaws.com",
  payload = "{}",
  headers = {
    ["content-type"] = "application/x-amz-json-1.1",
    ["x-amz-target"] = "AmazonEC2ContainerRegistry_V20150921.GetAuthorizationToken",
  },
})
-- Use headers with http.post/get
```

### Readonly and approval modes

Signing is pure computation and issues no request, so this module is never itself gated. What the
signature does affect is how the *request* you then send is classified: both modes read a signed AWS
call for the action it asks for rather than taking its verb at face value, and they admit an
enumerated list of read actions — the STS credential exchanges, ECR's `GetAuthorizationToken`, and
the reads `assay.aws.ec2` and `assay.aws.s3` issue. See [assay.aws.sts](aws-sts.md) for the list and
the rules.

So a hand-signed call to a service outside that list is refused under `--readonly` even when it is a
read, and prompts under approval:

```
readonly: http.get blocked (aws iam Action=ListRoles is not a read)
```

An explicit `classify: read` on a matching policy rule overrides the built-in, and is how to admit a
read the built-in list does not cover. A rule without `classify` leaves the built-in in charge.
