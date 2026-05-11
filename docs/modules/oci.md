---
category: Container & Registry
---

## oci

OCI container registry operations. Pull, push, copy, tag, and mutate images across registries. No
`require()` needed — available as a global builtin.

### Image operations

- `oci.copy(src, dst, opts?)` → true — Copy an image between registries. `opts` accepts
  `{src_auth, dst_auth}` for authentication. Each auth table can have `{username, password}` for
  Basic auth or `{token}` for bearer tokens.

- `oci.tag(src, new_tag)` → true — Tag an existing image under a new tag. Pulls from source
  registry, pushes to same registry with new tag.

- `oci.mutate(src, dst, files, opts?)` → true — Copy an image and add a new layer containing the
  provided files. `files` is a table of `{path = content}`. Creates the layer as a tar.gz.

Example:

```lua
-- Copy between registries
oci.copy(
  "registry.gitlab.com/group/project/image:abc123",
  "770508136720.dkr.ecr.us-east-1.amazonaws.com/app:abc123",
  {
    dst_auth = {token = ecr_token},
  }
)

-- Tag a release
oci.tag("registry.gitlab.com/group/project/image:abc123", "latest")

-- Add config file as a new layer
oci.mutate(
  "registry.gitlab.com/group/project/base:abc123",
  "770508136720.dkr.ecr.us-east-1.amazonaws.com/app:patched",
  {["/app/config.json"] = '{"env":"production"}'}
)
```
