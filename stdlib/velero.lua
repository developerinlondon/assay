--- @module assay.velero
--- @description Velero backup and restore. Backups, restores, schedules, storage locations.
--- @keywords velero, backups, restores, schedules, disaster-recovery, kubernetes
--- @quickref c:backups() -> [backup] | List backups
--- @quickref c:backup(name) -> backup|nil | Get backup by name
--- @quickref c:backup_status(name) -> {phase, errors, items_backed_up} | Get backup status
--- @quickref c:is_backup_completed(name) -> bool | Check if backup completed
--- @quickref c:is_backup_failed(name) -> bool | Check if backup failed
--- @quickref c:latest_backup(schedule_name) -> backup|nil | Get latest backup for schedule
--- @quickref c:restores() -> [restore] | List restores
--- @quickref c:restore(name) -> restore|nil | Get restore by name
--- @quickref c:restore_status(name) -> {phase, errors, warnings} | Get restore status
--- @quickref c:is_restore_completed(name) -> bool | Check if restore completed
--- @quickref c:schedules() -> [schedule] | List schedules
--- @quickref c:schedule(name) -> schedule|nil | Get schedule by name
--- @quickref c:schedule_status(name) -> {phase, last_backup} | Get schedule status
--- @quickref c:is_schedule_enabled(name) -> bool | Check if schedule is enabled
--- @quickref c:backup_storage_locations() -> [bsl] | List backup storage locations
--- @quickref c:backup_storage_location(name) -> bsl|nil | Get backup storage location
--- @quickref c:is_bsl_available(name) -> bool | Check if storage location is available
--- @quickref c:volume_snapshot_locations() -> [vsl] | List volume snapshot locations
--- @quickref c:volume_snapshot_location(name) -> vsl|nil | Get volume snapshot location
--- @quickref c:backup_repositories() -> [repo] | List backup repositories
--- @quickref c:backup_repository(name) -> repo|nil | Get backup repository
--- @quickref c:all_schedules_enabled() -> {enabled, disabled, total} | Check all schedules status
--- @quickref c:all_bsl_available() -> {available, unavailable, total} | Check all storage locations

local M = {}

local API_BASE = "/apis/velero.io/v1"

function M.client(url, token, namespace)
  local c = {
    url = url:gsub("/+$", ""),
    token = token,
    namespace = namespace or "velero",
  }

  local function headers(self)
    return { Authorization = "Bearer " .. self.token }
  end

  local function api_get(self, api_path)
    local resp = http.get(self.url .. api_path, { headers = headers(self) })
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("velero: GET " .. api_path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_list(self, api_path)
    local data = api_get(self, api_path)
    if not data then return {} end
    return data.items or {}
  end

  local function ns_path(self, resource, name)
    local p = API_BASE .. "/namespaces/" .. self.namespace .. "/" .. resource
    if name then
      p = p .. "/" .. name
    end
    return p
  end

  -- Backups

  function c:backups()
    return api_list(self, ns_path(self, "backups"))
  end

  function c:backup(name)
    return api_get(self, ns_path(self, "backups", name))
  end

  function c:backup_status(name)
    local b = self:backup(name)
    if not b then error("velero: backup " .. name .. " not found") end
    local status = b.status or {}
    return {
      phase = status.phase,
      started = status.startTimestamp,
      completed = status.completionTimestamp,
      expiration = status.expiration,
      errors = status.errors or 0,
      warnings = status.warnings or 0,
      items_backed_up = status.itemsBackedUp or 0,
      items_total = status.totalItems or 0,
    }
  end

  function c:is_backup_completed(name)
    local b = self:backup(name)
    if not b then return false end
    local status = b.status or {}
    return status.phase == "Completed"
  end

  function c:is_backup_failed(name)
    local b = self:backup(name)
    if not b then return false end
    local status = b.status or {}
    return status.phase == "Failed" or status.phase == "PartiallyFailed"
  end

  function c:latest_backup(schedule_name)
    local all = self:backups()
    local matching = {}
    for _, b in ipairs(all) do
      local labels = (b.metadata or {}).labels or {}
      if labels["velero.io/schedule-name"] == schedule_name then
        matching[#matching + 1] = b
      end
    end
    if #matching == 0 then return nil end
    table.sort(matching, function(a, b)
      local ts_a = (a.metadata or {}).creationTimestamp or ""
      local ts_b = (b.metadata or {}).creationTimestamp or ""
      return ts_a > ts_b
    end)
    return matching[1]
  end

  -- Restores

  function c:restores()
    return api_list(self, ns_path(self, "restores"))
  end

  function c:restore(name)
    return api_get(self, ns_path(self, "restores", name))
  end

  function c:restore_status(name)
    local r = self:restore(name)
    if not r then error("velero: restore " .. name .. " not found") end
    local status = r.status or {}
    return {
      phase = status.phase,
      started = status.startTimestamp,
      completed = status.completionTimestamp,
      errors = status.errors or 0,
      warnings = status.warnings or 0,
    }
  end

  function c:is_restore_completed(name)
    local r = self:restore(name)
    if not r then return false end
    local status = r.status or {}
    return status.phase == "Completed"
  end

  -- Schedules

  function c:schedules()
    return api_list(self, ns_path(self, "schedules"))
  end

  function c:schedule(name)
    return api_get(self, ns_path(self, "schedules", name))
  end

  function c:schedule_status(name)
    local s = self:schedule(name)
    if not s then error("velero: schedule " .. name .. " not found") end
    local status = s.status or {}
    return {
      phase = status.phase,
      last_backup = status.lastBackup,
      validation_errors = status.validationErrors or {},
    }
  end

  function c:is_schedule_enabled(name)
    local s = self:schedule(name)
    if not s then return false end
    local status = s.status or {}
    return status.phase == "Enabled"
  end

  -- Backup Storage Locations

  function c:backup_storage_locations()
    return api_list(self, ns_path(self, "backupstoragelocations"))
  end

  function c:backup_storage_location(name)
    return api_get(self, ns_path(self, "backupstoragelocations", name))
  end

  function c:is_bsl_available(name)
    local bsl = self:backup_storage_location(name)
    if not bsl then return false end
    local status = bsl.status or {}
    return status.phase == "Available"
  end

  -- Volume Snapshot Locations

  function c:volume_snapshot_locations()
    return api_list(self, ns_path(self, "volumesnapshotlocations"))
  end

  function c:volume_snapshot_location(name)
    return api_get(self, ns_path(self, "volumesnapshotlocations", name))
  end

  -- Backup Repositories

  function c:backup_repositories()
    return api_list(self, ns_path(self, "backuprepositories"))
  end

  function c:backup_repository(name)
    return api_get(self, ns_path(self, "backuprepositories", name))
  end

  -- Utilities

  function c:all_schedules_enabled()
    local all = self:schedules()
    local enabled = 0
    local disabled = 0
    local disabled_names = {}
    for _, s in ipairs(all) do
      local status = s.status or {}
      if status.phase == "Enabled" then
        enabled = enabled + 1
      else
        disabled = disabled + 1
        disabled_names[#disabled_names + 1] = (s.metadata or {}).name or "unknown"
      end
    end
    return {
      enabled = enabled,
      disabled = disabled,
      total = enabled + disabled,
      disabled_names = disabled_names,
    }
  end

  function c:all_bsl_available()
    local all = self:backup_storage_locations()
    local available = 0
    local unavailable = 0
    local unavailable_names = {}
    for _, bsl in ipairs(all) do
      local status = bsl.status or {}
      if status.phase == "Available" then
        available = available + 1
      else
        unavailable = unavailable + 1
        unavailable_names[#unavailable_names + 1] = (bsl.metadata or {}).name or "unknown"
      end
    end
    return {
      available = available,
      unavailable = unavailable,
      total = available + unavailable,
      unavailable_names = unavailable_names,
    }
  end

  return c
end

return M
