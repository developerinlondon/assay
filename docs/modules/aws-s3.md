---
category: Cloud & AWS
---

## assay.aws.s3

AWS S3 read-only client built on `assay.aws.sigv4`. Lists buckets and objects and fetches object
metadata via signed `GET` requests. Path-style addressing. Read-only — for uploads, copies, and
deletes use `assay.s3`, which covers the full object lifecycle.

### Client

- `s3.client(opts)` → client. `opts` is a table with the fields:
  - `access_key` _(optional)_ — AWS access key ID. Omit it and credentials are resolved through
    `assay.aws.sts` (profile / env / IRSA); pass it together with `secret_key`.
  - `secret_key` _(optional)_ — AWS secret access key
  - `region` _(required)_ — AWS region (or `AWS_REGION` / `AWS_DEFAULT_REGION`)
  - `profile` / `role_arn` _(optional)_ — passed through to `assay.aws.sts` when resolving
  - `session_token` _(optional)_ — STS session token
  - `endpoint` _(optional)_ — override the API endpoint (full URL or bare host). Defaults to
    `https://s3.<region>.amazonaws.com`. Useful for S3-compatible stores or a mock server in tests.

### Reads

- `c:list_buckets()` → `[{name, creation_date}]` — `GET /`.
- `c:list_objects(bucket, opts?)` → `{objects, is_truncated, next_continuation_token, key_count}` —
  `GET /<bucket>?list-type=2`. Each object: `{key, size, last_modified, etag}`. `opts`:
  `{prefix, max_keys, continuation_token}`.
- `c:head_object(bucket, key)` → `{status, headers}`|nil — object metadata via
  `GET /<bucket>/<key>`. Returns nil on 404.

### Mutation

None. Every method issues a signed `http.get`, so read-only and approval modes leave them
unrestricted.

Building the client is the part that can still be gated. Without explicit `access_key`/`secret_key`
it calls `sts.credentials`, and the IRSA and assume-role paths of that chain `POST` to STS — so a
readonly script fails with `readonly: http.post blocked` at `s3.client(...)`, before it issues a
single `GET`. Passing `access_key` + `secret_key` explicitly avoids the hop entirely. Relying on
`AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` only works when no profile is in play: the chain
consults `opts.profile` or `AWS_PROFILE` first, so if `AWS_PROFILE` names a `role_arn` or
`web_identity_token_file` profile, the env keys are never read and the STS POST still happens.

The alternative is a policy declaring the STS endpoint a read — but note that adding `http.rules` at
all switches the script to default-deny, so the reads above then need a rule of their own
(`hosts: ["s3.<region>.amazonaws.com"]`, `methods: [GET]`) or they are refused with
`policy: GET … is not allowed by any http rule`. Leave `paths` off that rule: bucket and object
reads span many paths, unlike the single `/` STS serves. See [assay.aws.sts](aws-sts.md) for a
worked two-rule policy.

Example:

```lua
local s3 = require("assay.aws.s3")
local c = s3.client({
  access_key = env.get("AWS_ACCESS_KEY_ID"),
  secret_key = env.get("AWS_SECRET_ACCESS_KEY"),
  region = "us-east-1",
})

for _, b in ipairs(c:list_buckets()) do
  print(b.name)
end

local listing = c:list_objects("demo-bucket", { prefix = "logs/", max_keys = 100 })
for _, obj in ipairs(listing.objects) do
  print(obj.key, obj.size)
end
```
