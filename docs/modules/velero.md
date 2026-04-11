## assay.velero

Velero backup and restore. Backups, restores, schedules, storage locations.
Client: `velero.client(url, token, namespace?)`. Default namespace: `"velero"`.

### Backups

- `c.backups:list()` -> [backup] -- List all backups
- `c.backups:get(name)` -> backup|nil -- Get backup by name
- `c.backups:status(name)` -> `{phase, started, completed, expiration, errors, warnings, items_backed_up, items_total}` -- Get status
- `c.backups:is_completed(name)` -> bool -- Check if backup phase is "Completed"
- `c.backups:is_failed(name)` -> bool -- Check if backup phase is "Failed" or "PartiallyFailed"
- `c.backups:latest(schedule_name)` -> backup|nil -- Get most recent backup for a schedule

### Restores

- `c.restores:list()` -> [restore] -- List all restores
- `c.restores:get(name)` -> restore|nil -- Get restore by name
- `c.restores:status(name)` -> `{phase, started, completed, errors, warnings}` -- Get restore status
- `c.restores:is_completed(name)` -> bool -- Check if restore phase is "Completed"

### Schedules

- `c.schedules:list()` -> [schedule] -- List all schedules
- `c.schedules:get(name)` -> schedule|nil -- Get schedule by name
- `c.schedules:status(name)` -> `{phase, last_backup, validation_errors}` -- Get schedule status
- `c.schedules:is_enabled(name)` -> bool -- Check if schedule phase is "Enabled"
- `c.schedules:all_enabled()` -> `{enabled, disabled, total, disabled_names}` -- Check all schedules

### Storage Locations

- `c.storage_locations:list()` -> [bsl] -- List backup storage locations
- `c.storage_locations:get(name)` -> bsl|nil -- Get backup storage location
- `c.storage_locations:is_available(name)` -> bool -- Check if storage location phase is "Available"
- `c.storage_locations:all_available()` -> `{available, unavailable, total, unavailable_names}` -- Check all storage locations

### Volume Snapshots

- `c.volume_snapshots:list()` -> [vsl] -- List volume snapshot locations
- `c.volume_snapshots:get(name)` -> vsl|nil -- Get volume snapshot location

### Backup Repositories

- `c.repositories:list()` -> [repo] -- List backup repositories
- `c.repositories:get(name)` -> repo|nil -- Get backup repository

Example:
```lua
local velero = require("assay.velero")
local c = velero.client("https://k8s-api:6443", env.get("K8S_TOKEN"), "velero")
local latest = c.backups:latest("daily-backup")
if latest then
  assert.eq(c.backups:is_completed(latest.metadata.name), true)
end
```
