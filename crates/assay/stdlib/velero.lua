--- @module assay.velero
--- @description Velero backup and restore. Backups, restores, schedules, storage locations.
--- @keywords velero, backups, restores, schedules, disaster-recovery, kubernetes, backup, restore, schedule, storage-location, snapshot, repository, completion, status, failover
--- @quickref c.backups:list() -> [backup] | List backups
--- @quickref c.backups:get(name) -> backup|nil | Get backup by name
--- @quickref c.backups:status(name) -> {phase, errors, items_backed_up} | Get backup status
--- @quickref c.backups:is_completed(name) -> bool | Check if backup completed
--- @quickref c.backups:is_failed(name) -> bool | Check if backup failed
--- @quickref c.backups:latest(schedule_name) -> backup|nil | Get latest backup for schedule
--- @quickref c.restores:list() -> [restore] | List restores
--- @quickref c.restores:get(name) -> restore|nil | Get restore by name
--- @quickref c.restores:status(name) -> {phase, errors, warnings} | Get restore status
--- @quickref c.restores:is_completed(name) -> bool | Check if restore completed
--- @quickref c.schedules:list() -> [schedule] | List schedules
--- @quickref c.schedules:get(name) -> schedule|nil | Get schedule by name
--- @quickref c.schedules:status(name) -> {phase, last_backup} | Get schedule status
--- @quickref c.schedules:is_enabled(name) -> bool | Check if schedule is enabled
--- @quickref c.schedules:all_enabled() -> {enabled, disabled, total} | Check all schedules status
--- @quickref c.storage_locations:list() -> [bsl] | List backup storage locations
--- @quickref c.storage_locations:get(name) -> bsl|nil | Get backup storage location
--- @quickref c.storage_locations:is_available(name) -> bool | Check if storage location is available
--- @quickref c.storage_locations:all_available() -> {available, unavailable, total} | Check all storage locations
--- @quickref c.volume_snapshots:list() -> [vsl] | List volume snapshot locations
--- @quickref c.volume_snapshots:get(name) -> vsl|nil | Get volume snapshot location
--- @quickref c.repositories:list() -> [repo] | List backup repositories
--- @quickref c.repositories:get(name) -> repo|nil | Get backup repository

local M = {}

local API_BASE = "/apis/velero.io/v1"

function M.client(url, token, namespace)
  local base_url = url:gsub("/+$", "")
  local ns = namespace or "velero"

  -- Shared HTTP helpers (captured by all sub-object methods as upvalues)

  local function headers()
    return { Authorization = "Bearer " .. token }
  end

  local function api_get(api_path)
    local resp = http.get(base_url .. api_path, { headers = headers() })
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("velero: GET " .. api_path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_list_items(api_path)
    local data = api_get(api_path)
    if not data then return {} end
    return data.items or {}
  end

  local function ns_path(resource, name)
    local p = API_BASE .. "/namespaces/" .. ns .. "/" .. resource
    if name then
      p = p .. "/" .. name
    end
    return p
  end

  -- ===== Client =====

  local c = {}

  -- ===== Backups =====

  c.backups = {}

  function c.backups:list()
    return api_list_items(ns_path("backups"))
  end

  function c.backups:get(name)
    return api_get(ns_path("backups", name))
  end

  function c.backups:status(name)
    local b = c.backups:get(name)
    if not b then error("velero: backup " .. name .. " not found") end
    local st = b.status or {}
    return {
      phase = st.phase,
      started = st.startTimestamp,
      completed = st.completionTimestamp,
      expiration = st.expiration,
      errors = st.errors or 0,
      warnings = st.warnings or 0,
      items_backed_up = st.itemsBackedUp or 0,
      items_total = st.totalItems or 0,
    }
  end

  function c.backups:is_completed(name)
    local b = c.backups:get(name)
    if not b then return false end
    local st = b.status or {}
    return st.phase == "Completed"
  end

  function c.backups:is_failed(name)
    local b = c.backups:get(name)
    if not b then return false end
    local st = b.status or {}
    return st.phase == "Failed" or st.phase == "PartiallyFailed"
  end

  function c.backups:latest(schedule_name)
    local all = c.backups:list()
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

  -- ===== Restores =====

  c.restores = {}

  function c.restores:list()
    return api_list_items(ns_path("restores"))
  end

  function c.restores:get(name)
    return api_get(ns_path("restores", name))
  end

  function c.restores:status(name)
    local r = c.restores:get(name)
    if not r then error("velero: restore " .. name .. " not found") end
    local st = r.status or {}
    return {
      phase = st.phase,
      started = st.startTimestamp,
      completed = st.completionTimestamp,
      errors = st.errors or 0,
      warnings = st.warnings or 0,
    }
  end

  function c.restores:is_completed(name)
    local r = c.restores:get(name)
    if not r then return false end
    local st = r.status or {}
    return st.phase == "Completed"
  end

  -- ===== Schedules =====

  c.schedules = {}

  function c.schedules:list()
    return api_list_items(ns_path("schedules"))
  end

  function c.schedules:get(name)
    return api_get(ns_path("schedules", name))
  end

  function c.schedules:status(name)
    local s = c.schedules:get(name)
    if not s then error("velero: schedule " .. name .. " not found") end
    local st = s.status or {}
    return {
      phase = st.phase,
      last_backup = st.lastBackup,
      validation_errors = st.validationErrors or {},
    }
  end

  function c.schedules:is_enabled(name)
    local s = c.schedules:get(name)
    if not s then return false end
    local st = s.status or {}
    return st.phase == "Enabled"
  end

  function c.schedules:all_enabled()
    local all = c.schedules:list()
    local enabled = 0
    local disabled = 0
    local disabled_names = {}
    for _, s in ipairs(all) do
      local st = s.status or {}
      if st.phase == "Enabled" then
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

  -- ===== Backup Storage Locations =====

  c.storage_locations = {}

  function c.storage_locations:list()
    return api_list_items(ns_path("backupstoragelocations"))
  end

  function c.storage_locations:get(name)
    return api_get(ns_path("backupstoragelocations", name))
  end

  function c.storage_locations:is_available(name)
    local bsl = c.storage_locations:get(name)
    if not bsl then return false end
    local st = bsl.status or {}
    return st.phase == "Available"
  end

  function c.storage_locations:all_available()
    local all = c.storage_locations:list()
    local available = 0
    local unavailable = 0
    local unavailable_names = {}
    for _, bsl in ipairs(all) do
      local st = bsl.status or {}
      if st.phase == "Available" then
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

  -- ===== Volume Snapshot Locations =====

  c.volume_snapshots = {}

  function c.volume_snapshots:list()
    return api_list_items(ns_path("volumesnapshotlocations"))
  end

  function c.volume_snapshots:get(name)
    return api_get(ns_path("volumesnapshotlocations", name))
  end

  -- ===== Backup Repositories =====

  c.repositories = {}

  function c.repositories:list()
    return api_list_items(ns_path("backuprepositories"))
  end

  function c.repositories:get(name)
    return api_get(ns_path("backuprepositories", name))
  end

  return c
end

return M
