## assay.velero

Velero backup and restore. Backups, restores, schedules, storage locations.
Client: `velero.client(url, token, namespace?)`. Default namespace: `"velero"`.

### Backups

- `c:backups()` → [backup] — List all backups
- `c:backup(name)` → backup|nil — Get backup by name
- `c:backup_status(name)` → `{phase, started, completed, expiration, errors, warnings, items_backed_up, items_total}` — Get status
- `c:is_backup_completed(name)` → bool — Check if backup phase is "Completed"
- `c:is_backup_failed(name)` → bool — Check if backup phase is "Failed" or "PartiallyFailed"
- `c:latest_backup(schedule_name)` → backup|nil — Get most recent backup for a schedule

### Restores

- `c:restores()` → [restore] — List all restores
- `c:restore(name)` → restore|nil — Get restore by name
- `c:restore_status(name)` → `{phase, started, completed, errors, warnings}` — Get restore status
- `c:is_restore_completed(name)` → bool — Check if restore phase is "Completed"

### Schedules

- `c:schedules()` → [schedule] — List all schedules
- `c:schedule(name)` → schedule|nil — Get schedule by name
- `c:schedule_status(name)` → `{phase, last_backup, validation_errors}` — Get schedule status
- `c:is_schedule_enabled(name)` → bool — Check if schedule phase is "Enabled"

### Storage Locations

- `c:backup_storage_locations()` → [bsl] — List backup storage locations
- `c:backup_storage_location(name)` → bsl|nil — Get backup storage location
- `c:is_bsl_available(name)` → bool — Check if storage location phase is "Available"
- `c:volume_snapshot_locations()` → [vsl] — List volume snapshot locations
- `c:volume_snapshot_location(name)` → vsl|nil — Get volume snapshot location

### Backup Repositories

- `c:backup_repositories()` → [repo] — List backup repositories
- `c:backup_repository(name)` → repo|nil — Get backup repository

### Utilities

- `c:all_schedules_enabled()` → `{enabled, disabled, total, disabled_names}` — Check all schedules
- `c:all_bsl_available()` → `{available, unavailable, total, unavailable_names}` — Check all storage locations

Example:
```lua
local velero = require("assay.velero")
local c = velero.client("https://k8s-api:6443", env.get("K8S_TOKEN"), "velero")
local latest = c:latest_backup("daily-backup")
if latest then
  assert.eq(c:is_backup_completed(latest.metadata.name), true)
end
```
