--- @module assay.kargo
--- @description Kargo continuous promotion. Stages, freight, promotions, warehouses, pipeline status.
--- @keywords kargo, promotions, stages, freight, warehouses, gitops, kubernetes, promotion, pipeline, health, wait, status, stage
--- @quickref c:stages(namespace) -> [stage] | List stages in namespace
--- @quickref c:stage(namespace, name) -> stage | Get stage by name
--- @quickref c:stage_status(namespace, name) -> {phase, freight, health} | Get stage status
--- @quickref c:is_stage_healthy(namespace, name) -> bool | Check if stage is healthy
--- @quickref c:wait_stage_healthy(namespace, name, timeout_secs?) -> true | Wait for stage health
--- @quickref c:freight_list(namespace, opts?) -> [freight] | List freight in namespace
--- @quickref c:freight(namespace, name) -> freight | Get freight by name
--- @quickref c:freight_status(namespace, name) -> status | Get freight status
--- @quickref c:promotions(namespace, opts?) -> [promotion] | List promotions
--- @quickref c:promotion(namespace, name) -> promotion | Get promotion by name
--- @quickref c:promotion_status(namespace, name) -> {phase, message, freight_id} | Get promotion status
--- @quickref c:promote(namespace, stage, freight) -> promotion | Create a promotion
--- @quickref c:warehouses(namespace) -> [warehouse] | List warehouses
--- @quickref c:warehouse(namespace, name) -> warehouse | Get warehouse by name
--- @quickref c:projects() -> [project] | List Kargo projects
--- @quickref c:project(name) -> project | Get project by name
--- @quickref c:pipeline_status(namespace) -> [{name, phase, freight, healthy}] | Get pipeline overview

local M = {}

local API_BASE = "/apis/kargo.akuity.io/v1alpha1"

function M.client(url, token)
  local c = {
    url = url:gsub("/+$", ""),
    token = token,
  }

  local function headers(self)
    return { Authorization = "Bearer " .. self.token }
  end

  local function api_get(self, api_path)
    local resp = http.get(self.url .. api_path, { headers = headers(self) })
    if resp.status ~= 200 then
      error("kargo: GET " .. api_path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_post(self, api_path, body)
    local resp = http.post(self.url .. api_path, body, { headers = headers(self) })
    if resp.status < 200 or resp.status >= 300 then
      error("kargo: POST " .. api_path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_put(self, api_path, body)
    local resp = http.put(self.url .. api_path, body, { headers = headers(self) })
    if resp.status < 200 or resp.status >= 300 then
      error("kargo: PUT " .. api_path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return json.parse(resp.body)
  end

  local function api_delete(self, api_path)
    local resp = http.delete(self.url .. api_path, { headers = headers(self) })
    if resp.status < 200 or resp.status >= 300 then
      error("kargo: DELETE " .. api_path .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
  end

  local function api_list(self, api_path)
    local data = api_get(self, api_path)
    return data.items or {}
  end

  function c:stages(namespace)
    return api_list(self, API_BASE .. "/namespaces/" .. namespace .. "/stages")
  end

  function c:stage(namespace, name)
    return api_get(self, API_BASE .. "/namespaces/" .. namespace .. "/stages/" .. name)
  end

  function c:stage_status(namespace, name)
    local s = self:stage(namespace, name)
    local status = s.status or {}
    return {
      phase = status.phase,
      current_freight_id = status.currentFreight and status.currentFreight.name or status.currentFreightId,
      health = status.health,
      conditions = status.conditions or {},
    }
  end

  function c:is_stage_healthy(namespace, name)
    local s = self:stage(namespace, name)
    local status = s.status or {}
    if status.phase == "Steady" then
      return true
    end
    local conditions = status.conditions or {}
    for i = #conditions, 1, -1 do
      if conditions[i].type == "Healthy" then
        return conditions[i].status == "True"
      end
    end
    return false
  end

  function c:wait_stage_healthy(namespace, name, timeout_secs)
    timeout_secs = timeout_secs or 60
    local interval = 5
    local elapsed = 0
    while elapsed < timeout_secs do
      local ok, healthy = pcall(self.is_stage_healthy, self, namespace, name)
      if ok and healthy then
        return true
      end
      sleep(interval)
      elapsed = elapsed + interval
    end
    error("kargo: stage " .. namespace .. "/" .. name .. " not healthy after " .. timeout_secs .. "s")
  end

  function c:freight_list(namespace, opts)
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
    return api_list(self, api_path)
  end

  function c:freight(namespace, name)
    return api_get(self, API_BASE .. "/namespaces/" .. namespace .. "/freight/" .. name)
  end

  function c:freight_status(namespace, name)
    local f = self:freight(namespace, name)
    return f.status or {}
  end

  function c:promotions(namespace, opts)
    opts = opts or {}
    local api_path = API_BASE .. "/namespaces/" .. namespace .. "/promotions"
    if opts.stage then
      api_path = api_path .. "?labelSelector=kargo.akuity.io/stage=" .. opts.stage
    end
    return api_list(self, api_path)
  end

  function c:promotion(namespace, name)
    return api_get(self, API_BASE .. "/namespaces/" .. namespace .. "/promotions/" .. name)
  end

  function c:promotion_status(namespace, name)
    local p = self:promotion(namespace, name)
    local status = p.status or {}
    return {
      phase = status.phase,
      message = status.message,
      freight_id = status.freight and status.freight.name or status.freightId,
    }
  end

  function c:promote(namespace, stage, freight)
    local body = {
      apiVersion = "kargo.akuity.io/v1alpha1",
      kind = "Promotion",
      metadata = {
        namespace = namespace,
        generateName = stage .. "-",
      },
      spec = {
        stage = stage,
        freight = freight,
      },
    }
    return api_post(self, API_BASE .. "/namespaces/" .. namespace .. "/promotions", body)
  end

  function c:warehouses(namespace)
    return api_list(self, API_BASE .. "/namespaces/" .. namespace .. "/warehouses")
  end

  function c:warehouse(namespace, name)
    return api_get(self, API_BASE .. "/namespaces/" .. namespace .. "/warehouses/" .. name)
  end

  function c:projects()
    return api_list(self, API_BASE .. "/projects")
  end

  function c:project(name)
    return api_get(self, API_BASE .. "/projects/" .. name)
  end

  function c:pipeline_status(namespace)
    local stage_list = self:stages(namespace)
    local result = {}
    for _, s in ipairs(stage_list) do
      local status = s.status or {}
      local healthy = false
      if status.phase == "Steady" then
        healthy = true
      else
        local conditions = status.conditions or {}
        for i = #conditions, 1, -1 do
          if conditions[i].type == "Healthy" then
            healthy = conditions[i].status == "True"
            break
          end
        end
      end
      result[#result + 1] = {
        name = s.metadata.name,
        phase = status.phase,
        freight = status.currentFreight and status.currentFreight.name or status.currentFreightId,
        healthy = healthy,
      }
    end
    return result
  end

  return c
end

return M
