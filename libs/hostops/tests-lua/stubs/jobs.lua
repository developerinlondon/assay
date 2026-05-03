--! Stub `jobs` service — no-op tracker.

local M = {}

local jobs = {}
local seq = 0

function M.start(spec)
  seq = seq + 1
  local id = "job-" .. tostring(seq)
  jobs[id] = {
    id = id, kind = spec.kind, target = spec.target,
    name = spec.name, stages = spec.stages or {},
    state = "running", logs = {},
  }
  return jobs[id]
end

function M.update_stage(id, stage, status, msg)
  local j = jobs[id]
  if not j then return end
  for _, s in ipairs(j.stages) do
    if s.id == stage then
      s.status = status; s.message = msg
    end
  end
end

function M.append_log(id, msg)
  local j = jobs[id]
  if j then table.insert(j.logs, { ts = os.time(), msg = msg }) end
end

function M.fail(id, err)
  local j = jobs[id]
  if j then j.state = "failed"; j.error = err end
end

function M.complete(id, result)
  local j = jobs[id]
  if j then j.state = "completed"; j.result = result end
end

function M.active(filter)
  filter = filter or {}
  local out = {}
  for _, j in pairs(jobs) do
    if j.state == "running" then
      if not filter.kind or j.kind == filter.kind then
        out[#out+1] = j
      end
    end
  end
  return out
end

function M.get(id) return jobs[id] end

return M
