---
category: Container & Registry
---

## oci

OCI container registry operations. Pull, push, copy, tag, and mutate images across registries. No
`require()` needed — available as a global builtin.

### Image operations

- `oci.copy(src, dst, opts?)` → true — Copy an image between registries. Pulls each layer one at a
  time from `src` and pushes to `dst`; peak memory is the size of the largest single layer. `opts`
  accepts `{src_auth, dst_auth}` where each auth table is `{username, password}`.

- `oci.tag(src, new_tag, opts?)` → true — Re-tag an image **within the same registry+repository** by
  pulling the manifest and pushing it under `new_tag`. No layer data is copied — the new tag points
  at the existing blobs. `opts` accepts `{auth = {username, password}}`. For cross-registry tagging,
  use `oci.copy`.

- `oci.mutate(src, dst, files, opts?)` → true — Copy `src` to `dst` and append a new tar.gz layer
  containing `files` (a table of `{path = content}`). The image config is updated so
  `rootfs.diff_ids` includes the new layer, matching `crane mutate` semantics. `opts` accepts
  `{src_auth, dst_auth}`.

Example:

```lua
-- Copy between registries (streams layer-by-layer)
oci.copy(
  "registry.gitlab.com/group/project/image:abc123",
  "770508136720.dkr.ecr.us-east-1.amazonaws.com/app:abc123",
  {
    src_auth = { username = env.get("CI_REGISTRY_USER"), password = env.get("CI_REGISTRY_PASSWORD") },
    dst_auth = { username = "AWS",                       password = ecr_token },
  }
)

-- Re-tag within the same repository (cheap manifest-only push)
oci.tag(
  "770508136720.dkr.ecr.us-east-1.amazonaws.com/app:abc123",
  "latest",
  { auth = { username = "AWS", password = ecr_token } }
)

-- Append a config file as a new layer
oci.mutate(
  "registry.gitlab.com/group/project/base:abc123",
  "770508136720.dkr.ecr.us-east-1.amazonaws.com/app:patched",
  { ["app/config.json"] = '{"env":"production"}' },
  {
    src_auth = { username = env.get("CI_REGISTRY_USER"), password = env.get("CI_REGISTRY_PASSWORD") },
    dst_auth = { username = "AWS",                       password = ecr_token },
  }
)
```
