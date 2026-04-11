## assay.k8s

Kubernetes API client. 30+ resource types, CRDs, readiness checks, pod logs, rollouts.
Module-level functions: auto-discovers cluster API via `KUBERNETES_SERVICE_HOST` env var.
Auth: uses service account token from `/var/run/secrets/kubernetes.io/serviceaccount/token`.
All functions accept optional `opts` with `{base_url, token}` overrides.

Supported kinds: pod, service, secret, configmap, endpoints, serviceaccount, persistentvolumeclaim (pvc),
limitrange, resourcequota, event, namespace, node, persistentvolume (pv), deployment, statefulset,
daemonset, replicaset, job, cronjob, ingress, ingressclass, networkpolicy, storageclass, role,
rolebinding, clusterrole, clusterrolebinding, hpa, poddisruptionbudget (pdb).

- `M.register_crd(kind, api_group, version, plural, cluster_scoped?)` → nil — Register custom resource for use with get/list/create
- `M.get(path, opts?)` → resource — Raw GET any K8s API path
- `M.post(path, body, opts?)` → resource — Raw POST to any K8s API path
- `M.put(path, body, opts?)` → resource — Raw PUT to any K8s API path
- `M.patch(path, body, opts?)` → resource — Raw PATCH any K8s API path. `opts.content_type` defaults to merge-patch.
- `M.delete(path, opts?)` → nil — Raw DELETE any K8s API path
- `M.get_resource(namespace, kind, name, opts?)` → resource — Get resource by kind and name
- `M.list(namespace, kind, opts?)` → `{items}` — List resources. `opts`: `{label_selector, field_selector, limit}`
- `M.create(namespace, kind, body, opts?)` → resource — Create resource
- `M.update(namespace, kind, name, body, opts?)` → resource — Replace resource
- `M.patch_resource(namespace, kind, name, body, opts?)` → resource — Patch resource
- `M.delete_resource(namespace, kind, name, opts?)` → nil — Delete resource
- `M.exists(namespace, kind, name, opts?)` → bool — Check if resource exists
- `M.get_secret(namespace, name, opts?)` → `{key=value}` — Get decoded secret data (base64-decoded)
- `M.get_configmap(namespace, name, opts?)` → `{key=value}` — Get ConfigMap data
- `M.list_pods(namespace, opts?)` → `{items}` — List pods in namespace
- `M.list_events(namespace, opts?)` → `{items}` — List events in namespace
- `M.pod_status(namespace, opts?)` → `{running, pending, succeeded, failed, unknown, total}` — Get pod status counts
- `M.is_ready(namespace, kind, name, opts?)` → bool — Check if resource is ready (deployment, statefulset, daemonset, job, node)
- `M.wait_ready(namespace, kind, name, timeout_secs?, opts?)` → true — Wait for readiness, errors on timeout. Default 60s.
- `M.service_endpoints(namespace, name, opts?)` → [ip] — Get service endpoint IP addresses
- `M.logs(namespace, pod_name, opts?)` → string — Get pod logs. `opts`: `{tail, container, previous, since}`
- `M.rollout_status(namespace, name, opts?)` → `{desired, updated, ready, available, unavailable, complete}` — Get deployment rollout status
- `M.node_status(opts?)` → `[{name, ready, roles, capacity, allocatable}]` — Get all node statuses
- `M.namespace_exists(name, opts?)` → bool — Check if namespace exists
- `M.events_for(namespace, kind, name, opts?)` → `{items}` — Get events for a specific resource

Example:
```lua
local k8s = require("assay.k8s")
k8s.wait_ready("default", "deployment", "my-app", 120)
local secret = k8s.get_secret("default", "my-secret")
log.info("DB password: " .. secret["password"])
```
