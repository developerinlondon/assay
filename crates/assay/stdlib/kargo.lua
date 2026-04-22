--- @module assay.kargo
--- @description Kargo continuous promotion. Stages, freight, promotions, warehouses, pipeline status.
--- @keywords kargo, promotions, stages, freight, warehouses, gitops, kubernetes, promotion, pipeline, health, wait, status, stage
--- @quickref c.stages:list(namespace) -> [stage] | List stages in namespace
--- @quickref c.stages:get(namespace, name) -> stage | Get stage by name
--- @quickref c.stages:status(namespace, name) -> {phase, freight, health} | Get stage status
--- @quickref c.stages:is_healthy(namespace, name) -> bool | Check if stage is healthy
--- @quickref c.stages:wait_healthy(namespace, name, timeout_secs?) -> true | Wait for stage health
--- @quickref c.stages:pipeline_status(namespace) -> [{name, phase, freight, healthy}] | Get pipeline overview
--- @quickref c.freight:list(namespace, opts?) -> [freight] | List freight in namespace
--- @quickref c.freight:get(namespace, name) -> freight | Get freight by name
--- @quickref c.freight:status(namespace, name) -> status | Get freight status
--- @quickref c.promotions:list(namespace, opts?) -> [promotion] | List promotions
--- @quickref c.promotions:get(namespace, name) -> promotion | Get promotion by name
--- @quickref c.promotions:status(namespace, name) -> {phase, message, freight_id} | Get promotion status
--- @quickref c.promotions:create(namespace, stage, freight) -> promotion | Create a promotion
--- @quickref c.warehouses:list(namespace) -> [warehouse] | List warehouses
--- @quickref c.warehouses:get(namespace, name) -> warehouse | Get warehouse by name
--- @quickref c.projects:list() -> [project] | List Kargo projects
--- @quickref c.projects:get(name) -> project | Get project by name

local M = {}

local API_BASE = "/apis/kargo.akuity.io/v1alpha1"

function M.client(url, token)
  local base_url = url:gsub("/+$", "")

  -- Shared HTTP helpers (captured by all sub-object methods as upvalues)

  local function headers()
    return { Authorization = "Bearer " .. token }
  end

  local function api_get(api_path)
    local resp = http.get(base_url .. api_path, { headers = headers() })
    if resp.status ~= 200 then
      error("kargo: GET " .. api_path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_post(api_path, body)
    local resp = http.post(base_url .. api_path, body, { headers = headers() })
    if resp.status < 200 or resp.status >= 300 then
      error("kargo: POST " .. api_path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_list_items(api_path)
    local data = api_get(api_path)
    return data.items or {}
  end

  -- ===== Client =====

  local c = {}

  -- ===== Stages =====

  c.stages = {}

  function c.stages:list(namespace)
    return api_list_items(API_BASE .. "/namespaces/" .. namespace .. "/stages")
  end

  function c.stages:get(namespace, name)
    return api_get(API_BASE .. "/namespaces/" .. namespace .. "/stages/" .. name)
  end

  function c.stages:status(namespace, name)
    local s = c.stages:get(namespace, name)
    local st = s.status or {}
    return {
      phase = st.phase,
      current_freight_id = st.currentFreight and st.currentFreight.name or st.currentFreightId,
      health = st.health,
      conditions = st.conditions or {},
    }
  end

  function c.stages:is_healthy(namespace, name)
    local s = c.stages:get(namespace, name)
    local st = s.status or {}
    if st.phase == "Steady" then
      return true
    end
    local conditions = st.conditions or {}
    for i = #conditions, 1, -1 do
      if conditions[i].type == "Healthy" then
        return conditions[i].status == "True"
      end
    end
    return false
  end

  function c.stages:wait_healthy(namespace, name, timeout_secs)
    timeout_secs = timeout_secs or 60
    local interval = 5
    local elapsed = 0
    while elapsed < timeout_secs do
      local ok, healthy = pcall(c.stages.is_healthy, c.stages, namespace, name)
      if ok and healthy then
        return true
      end
      sleep(interval)
      elapsed = elapsed + interval
    end
    error("kargo: stage " .. namespace .. "/" .. name .. " not healthy after " .. timeout_secs .. "s")
  end

  function c.stages:pipeline_status(namespace)
    local stage_list = c.stages:list(namespace)
    local result = {}
    for _, s in ipairs(stage_list) do
      local st = s.status or {}
      local healthy = false
      if st.phase == "Steady" then
        healthy = true
      else
        local conditions = st.conditions or {}
        for i = #conditions, 1, -1 do
          if conditions[i].type == "Healthy" then
            healthy = conditions[i].status == "True"
            break
          end
        end
      end
      result[#result + 1] = {
        name = s.metadata.name,
        phase = st.phase,
        freight = st.currentFreight and st.currentFreight.name or st.currentFreightId,
        healthy = healthy,
      }
    end
    return result
  end

  -- ===== Freight =====

  c.freight = {}

  function c.freight:list(namespace, opts)
    opts = opts or {}
    local api_path = API_BASE .. "/namespaces/" .. namespace .. "/freight"
    local params = {}
    if opts.stage then
      params[#params + 1] = "labelSelector=kargo.akuity.io/stage=" .. opts.stage
    end
    if opts.warehouse then
      params[#params + 1] = "labelSelector=kargo.akuity.io/warehouse=" .. opts.warehouse
    end
    if #params > 0 then
      api_path = api_path .. "?" .. table.concat(params, "&")
    end
    return api_list_items(api_path)
  end

  function c.freight:get(namespace, name)
    return api_get(API_BASE .. "/namespaces/" .. namespace .. "/freight/" .. name)
  end

  function c.freight:status(namespace, name)
    local f = c.freight:get(namespace, name)
    return f.status or {}
  end

  -- ===== Promotions =====

  c.promotions = {}

  function c.promotions:list(namespace, opts)
    opts = opts or {}
    local api_path = API_BASE .. "/namespaces/" .. namespace .. "/promotions"
    if opts.stage then
      api_path = api_path .. "?labelSelector=kargo.akuity.io/stage=" .. opts.stage
    end
    return api_list_items(api_path)
  end

  function c.promotions:get(namespace, name)
    return api_get(API_BASE .. "/namespaces/" .. namespace .. "/promotions/" .. name)
  end

  function c.promotions:status(namespace, name)
    local p = c.promotions:get(namespace, name)
    local st = p.status or {}
    return {
      phase = st.phase,
      message = st.message,
      freight_id = st.freight and st.freight.name or st.freightId,
    }
  end

  function c.promotions:create(namespace, stage, freight_ref)
    local body = {
      apiVersion = "kargo.akuity.io/v1alpha1",
      kind = "Promotion",
      metadata = {
        namespace = namespace,
        generateName = stage .. "-",
      },
      spec = {
        stage = stage,
        freight = freight_ref,
      },
    }
    return api_post(API_BASE .. "/namespaces/" .. namespace .. "/promotions", body)
  end

  -- ===== Warehouses =====

  c.warehouses = {}

  function c.warehouses:list(namespace)
    return api_list_items(API_BASE .. "/namespaces/" .. namespace .. "/warehouses")
  end

  function c.warehouses:get(namespace, name)
    return api_get(API_BASE .. "/namespaces/" .. namespace .. "/warehouses/" .. name)
  end

  -- ===== Projects =====

  c.projects = {}

  function c.projects:list()
    return api_list_items(API_BASE .. "/projects")
  end

  function c.projects:get(name)
    return api_get(API_BASE .. "/projects/" .. name)
  end

  return c
end

return M
