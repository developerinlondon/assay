## assay.k8s

Kubernetes API client. 30+ resource types, CRDs, readiness checks, pod logs, rollouts.
Module-level functions: auto-discovers cluster API via `KUBERNETES_SERVICE_HOST` env var.
Auth: uses service account token from `/var/run/secrets/kubernetes.io/serviceaccount/token`.
All functions accept optional `opts` with `{base_url, token}` overrides.

Supported kinds: pod, service, secret, configmap, endpoints, serviceaccount, persistentvolumeclaim (pvc),
limitrange, resourcequota, event, namespace, node, persistentvolume (pv), deployment, statefulset,
daemonset, replicaset, job, cronjob, ingress, ingressclass, networkpolicy, storageclass, role,
rolebinding, clusterrole, clusterrolebinding, hpa, poddisruptionbudget (pdb).

### CRD Registration

- `M.register_crd(kind, api_group, version, plural, cluster_scoped?)` — Register custom resource for use with get/list/create

### Raw HTTP Verbs

- `M.get(path, opts?)` → resource — Raw GET any K8s API path
- `M.post(path, body, opts?)` → resource — Raw POST to any K8s API path
- `M.put(path, body, opts?)` → resource — Raw PUT to any K8s API path
- `M.patch(path, body, opts?)` → resource — Raw PATCH any K8s API path. `opts.content_type` defaults to merge-patch.
- `M.delete(path, opts?)` → nil — Raw DELETE any K8s API path

### Resources (`M.resources`)

Generic CRUD operations for any resource kind.

- `M.resources:get(namespace, kind, name, opts?)` → resource — Get resource by kind and name
- `M.resources:list(namespace, kind, opts?)` → `{items}` — List resources. `opts`: `{label_selector, field_selector, limit}`
- `M.resources:create(namespace, kind, body, opts?)` → resource — Create resource
- `M.resources:update(namespace, kind, name, body, opts?)` → resource — Replace resource
- `M.resources:patch(namespace, kind, name, body, opts?)` → resource — Patch resource
- `M.resources:delete(namespace, kind, name, opts?)` → nil — Delete resource
- `M.resources:exists(namespace, kind, name, opts?)` → bool — Check if resource exists
- `M.resources:is_ready(namespace, kind, name, opts?)` → bool — Check if resource is ready (deployment, statefulset, daemonset, job, node)
- `M.resources:wait_ready(namespace, kind, name, timeout_secs?, opts?)` → true — Wait for readiness, errors on timeout. Default 60s.

### Secrets (`M.secrets`)

- `M.secrets:get(namespace, name, opts?)` → `{key=value}` — Get decoded secret data (base64-decoded)

### ConfigMaps (`M.configmaps`)

- `M.configmaps:get(namespace, name, opts?)` → `{key=value}` — Get ConfigMap data

### Pods (`M.pods`)

- `M.pods:list(namespace, opts?)` → `{items}` — List pods in namespace
- `M.pods:status(namespace, opts?)` → `{running, pending, succeeded, failed, unknown, total}` — Get pod status counts
- `M.pods:logs(namespace, pod_name, opts?)` → string — Get pod logs. `opts`: `{tail, container, previous, since}`

### Services (`M.services`)

- `M.services:endpoints(namespace, name, opts?)` → [ip] — Get service endpoint IP addresses

### Deployments (`M.deployments`)

- `M.deployments:rollout_status(namespace, name, opts?)` → `{desired, updated, ready, available, unavailable, complete}` — Get deployment rollout status

### Nodes (`M.nodes`)

- `M.nodes:status(opts?)` → `[{name, ready, roles, capacity, allocatable}]` — Get all node statuses

### Namespaces (`M.namespaces`)

- `M.namespaces:exists(name, opts?)` → bool — Check if namespace exists

### Events (`M.events`)

- `M.events:list(namespace, opts?)` → `{items}` — List events in namespace
- `M.events:for_resource(namespace, kind, name, opts?)` → `{items}` — Get events for a specific resource

### Backward Compatibility

All legacy flat functions (`M.get_resource`, `M.list`, `M.get_secret`, `M.pod_status`, etc.) remain available and delegate to the sub-objects above.

Example:
```lua
local k8s = require("assay.k8s")

-- New sub-object style
k8s.resources:wait_ready("default", "deployment", "my-app", 120)
local secret = k8s.secrets:get("default", "my-secret")
log.info("DB password: " .. secret["password"])

-- Legacy style still works
k8s.wait_ready("default", "deployment", "my-app", 120)
local secret = k8s.get_secret("default", "my-secret")
```
