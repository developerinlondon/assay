---
category: Cloud & AWS
---

## assay.aws.s3

AWS S3 read-only client built on `assay.aws.sigv4`. Lists buckets and objects and fetches object
metadata via signed `GET` requests. Path-style addressing. Read-only — for uploads, copies, and
deletes use `assay.s3`, which covers the full object lifecycle.

### Client

- `s3.client(opts)` → client. `opts` is a table with the fields:
  - `access_key` _(required)_ — AWS access key ID
  - `secret_key` _(required)_ — AWS secret access key
  - `region` _(required)_ — AWS region
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
