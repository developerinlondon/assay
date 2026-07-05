---
category: Cloud & AWS
---

## assay.aws.ec2

AWS EC2 read-only client built on `assay.aws.sigv4`. Describes instances, EBS volumes, and security
groups via the EC2 Query API (signed `GET` requests, XML responses). Read-only — no mutating
actions.

### Client

- `ec2.client(opts)` → client. `opts` is a table with the fields:
  - `access_key` _(required)_ — AWS access key ID
  - `secret_key` _(required)_ — AWS secret access key
  - `region` _(required)_ — AWS region
  - `session_token` _(optional)_ — STS session token
  - `endpoint` _(optional)_ — override the API endpoint (full URL or bare host). Defaults to
    `https://ec2.<region>.amazonaws.com`. Useful for VPC endpoints or for injecting a mock server in
    tests.

### Describe

- `c:describe_instances(opts?)` → `[instance]` — `Action=DescribeInstances`. Each instance:
  `{instance_id, instance_type, state, private_ip, public_ip, availability_zone}`.
- `c:describe_volumes(opts?)` → `[volume]` — `Action=DescribeVolumes`. Each volume:
  `{volume_id, size, state, availability_zone, volume_type}`.
- `c:describe_security_groups(opts?)` → `[group]` — `Action=DescribeSecurityGroups`. Each group:
  `{group_id, group_name, description, vpc_id}`.

All three accept an optional `opts` table:

- `instance_ids` / `volume_ids` / `group_ids` — a list of IDs to restrict the query.
- `filters` — a list of `{name = "...", values = {...}}` EC2 filters, e.g.
  `{{ name = "instance-state-name", values = { "running" } }}`.

### Mutation

None. Every method issues a signed `http.get`, so read-only and approval modes leave them
unrestricted.

Example:

```lua
local ec2 = require("assay.aws.ec2")
local c = ec2.client({
  access_key = env.get("AWS_ACCESS_KEY_ID"),
  secret_key = env.get("AWS_SECRET_ACCESS_KEY"),
  session_token = env.get("AWS_SESSION_TOKEN"),
  region = "us-east-1",
})

local running = c:describe_instances({
  filters = { { name = "instance-state-name", values = { "running" } } },
})
for _, i in ipairs(running) do
  print(i.instance_id, i.instance_type, i.availability_zone)
end
```
