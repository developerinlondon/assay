local state = require("services.state")
local audit = require("services.audit")

local M = {}

local function actor(req)
  local h = req.headers or {}
  return h["Cf-Access-Authenticated-User-Email"]
      or h["cf-access-authenticated-user-email"]
      or "local-dev"
end

local function ip_of(req)
  local h = req.headers or {}
  return h["Cf-Connecting-Ip"] or h["cf-connecting-ip"] or req.remote_addr or "?"
end

-- Distinct semantics:
--   restart   — bounce the systemd-nspawn unit (host-side stop+start of the container)
--   reboot    — signal init INSIDE the container to reboot (container-side reset)
--   poweroff  — graceful shutdown of the container OS
--   terminate — SIGKILL the container (forceful)
--   start/stop — host-side unit start/stop
local function unit_for(name) return "systemd-nspawn@" .. name .. ".service" end

local ACTIONS = {
  start     = function(name) return systemd.start(unit_for(name)) end,
  stop      = function(name) return systemd.stop(unit_for(name)) end,
  restart   = function(name) return systemd.restart(unit_for(name)) end,
  reboot    = function(name) return systemd.machine_reboot(name) end,
  poweroff  = function(name) return systemd.machine_poweroff(name) end,
  terminate = function(name) return systemd.machine_terminate(name) end,
}

-- Reduce verbose D-Bus / polkit / mlua error messages down to a one-line
-- explanation suitable for an inline pill.
local function clean_err(s)
  s = tostring(s or "")
  local first = s:match("([^\n]+)") or s
  first = first:gsub("%[string %\"%?%\"%]:%d+:%s*", "")
  first = first:gsub("^runtime error:%s*", "")
  first = first:gsub("^systemd%.[^:]+:%s*", "")
  -- Friendly aliases for known D-Bus error names
  if first:find("InteractiveAuthorizationRequired", 1, true) then
    return "polkit denied (no rule for this user)"
  end
  if first:find("AccessDenied", 1, true) then
    return "polkit denied (access denied)"
  end
  if first:find("NoSuchUnit", 1, true) then
    return "no such systemd unit"
  end
  if first:find("NoSuchMachine", 1, true) then
    return "no such machine"
  end
  if #first > 160 then first = first:sub(1, 157) .. "…" end
  return first
end

function M.handle(req)
  local name, action = (req.path or ""):match("^/api/machines/([^/]+)/([^/]+)$")
  if not name or not action then
    return { status = 404, body = "not found" }
  end
  -- "destroy" and "resources" live outside the lifecycle ACTIONS map
  -- (different shapes — destroy tears down the machine, resources edits
  -- the live + persistent CPU/memory drop-in).
  if action == "destroy" then
    return M.destroy(req)
  end
  if action == "resources" then
    return M.resources(req)
  end
  if not ACTIONS[action] then
    return { status = 404, body = "not found" }
  end

  local ok, err = pcall(ACTIONS[action], name)
  state.bump()

  audit.append({
    actor  = actor(req),
    ip     = ip_of(req),
    action = "machine." .. action,
    target = name,
    result = ok and "ok" or "fail",
    reason = ok and nil or tostring(err),
  })

  -- Always return 200 with the pill body so HTMX swaps reliably (htmx
  -- skips swap on 4xx/5xx by default). The audit log + pill class carry
  -- the real success/failure signal.
  return {
    status = 200,
    headers = {
      ["HX-Trigger"]   = "dashboard-refresh",
      ["Content-Type"] = "text/html; charset=utf-8",
    },
    body = ok
      and ('<span class="pill pill-ok">' .. action .. " OK</span>")
      or  ('<span class="pill pill-err" title="' .. clean_err(err)
            .. '">' .. action .. " FAIL: " .. clean_err(err) .. "</span>"),
  }
end

-- ── Provision new machine (POST /api/machines) ──────────────────────────
--
-- Form fields: name, template
-- Browser form submissions get a 303 redirect with a flash banner
-- (kind=ok|warn|err, msg=...). API callers (Accept: application/json) get
-- the structured result.

local form    = require("pages.form")
local provsvc = require("services.nspawn.provision")

local function urlencode(s)
  return (s:gsub("([^%w%-_.~])", function(c)
    return string.format("%%%02X", c:byte())
  end))
end

local function wants_redirect(req)
  local accept = (req.headers and (req.headers["Accept"] or req.headers["accept"])) or ""
  if accept:find("application/json", 1, true) then return false end
  local ct = (req.headers and (req.headers["Content-Type"] or req.headers["content-type"])) or ""
  return ct:find("application/x-www-form-urlencoded", 1, true) ~= nil
end

local function json_response(status, body)
  return {
    status  = status,
    headers = { ["Content-Type"] = "application/json" },
    body    = json.encode(body),
  }
end

local jobs = require("services.nspawn.jobs")

local function parse_optional_positive_number(s, max)
  if s == nil or s == "" then return nil, nil end
  local n = tonumber(s)
  if not n then return nil, "must be a number" end
  if n <= 0 then return nil, "must be > 0" end
  if max and n > max then return nil, ("must be ≤ " .. tostring(max)) end
  return n, nil
end

function M.provision(req)
  local body = form.parse(req)
  local name = body.name
  local tmpl = body.template

  -- Quick validation BEFORE we spawn — let bad input return synchronously
  -- so the form can show an inline error rather than a phantom job card.
  if type(name) ~= "string" or name == ""
     or not name:match("^[A-Za-z0-9._%-]+$") then
    if wants_redirect(req) then
      return {
        status = 303,
        headers = {
          ["Location"] = "/machines/new?kind=err&msg=" ..
                         urlencode("name must match [A-Za-z0-9._-]+"),
        },
      }
    end
    return json_response(400, { ok = false, error = "invalid name" })
  end
  if type(tmpl) ~= "string" or tmpl == "" then
    if wants_redirect(req) then
      return {
        status = 303,
        headers = { ["Location"] = "/machines/new?kind=err&msg=" .. urlencode("template required") },
      }
    end
    return json_response(400, { ok = false, error = "template required" })
  end

  -- Optional resource limits.
  local cpu_cores, cpu_err = parse_optional_positive_number(body.cpu_cores, 256)
  if cpu_err then
    if wants_redirect(req) then
      return { status = 303,
        headers = { ["Location"] = "/machines/new?kind=err&msg=" ..
                                   urlencode("cpu_cores: " .. cpu_err) } }
    end
    return json_response(400, { ok = false, error = "cpu_cores: " .. cpu_err })
  end
  local memory_gb, mem_err = parse_optional_positive_number(body.memory_gb, 1024)
  if mem_err then
    if wants_redirect(req) then
      return { status = 303,
        headers = { ["Location"] = "/machines/new?kind=err&msg=" ..
                                   urlencode("memory_gb: " .. mem_err) } }
    end
    return json_response(400, { ok = false, error = "memory_gb: " .. mem_err })
  end

  -- Create the job entry and spawn the actual provision in the background.
  -- All real work (existence check, debootstrap, etc.) lives in the spawned
  -- coroutine so the HTTP request returns in milliseconds. If the name is
  -- already taken, the job transitions to "failed" and the UI surfaces the
  -- error in the in-flight card.
  local job = jobs.start({ name = name, template = tmpl })
  local actor_name = actor(req)

  async.spawn(function()
    local ok_call, ret = pcall(provsvc.provision, {
      name = name, template = tmpl, actor = actor_name,
      cpu_cores = cpu_cores, memory_gb = memory_gb,
      on_stage = function(stage, status, msg)
        jobs.update_stage(job.id, stage, status, msg)
      end,
    })
    if ok_call and ret and ret.ok then
      jobs.complete(job.id, ret)
    else
      local err
      if not ok_call then err = tostring(ret)
      elseif ret then     err = ret.error or "unknown"
      else                err = "unknown" end
      jobs.fail(job.id, err)
    end
  end)

  if wants_redirect(req) then
    return {
      status = 303,
      headers = { ["Location"] = "/machines?provisioning=" .. job.id },
    }
  end
  return json_response(202, { ok = true, job_id = job.id, status = "running" })
end

--- GET /api/machines/jobs/<id>  → JSON job state
function M.job_status(req)
  local id = (req.path or ""):match("^/api/machines/jobs/(.+)$")
  if not id then return json_response(400, { error = "id required" }) end
  local job = jobs.get(id)
  if not job then return json_response(404, { error = "no such job" }) end
  return json_response(200, job)
end

local function is_htmx(req)
  local h = req.headers or {}
  return (h["HX-Request"] or h["hx-request"]) == "true"
end

function M.destroy(req)
  -- POST /api/machines/<name>/destroy  (form-friendly, mirrors the
  -- existing /api/machines/<name>/<action> shape rather than DELETE).
  local name = (req.path or ""):match("^/api/machines/([^/]+)/destroy$")
  if not name then return json_response(400, { error = "name required" }) end

  local result = provsvc.destroy({ name = name, actor = actor(req) })

  -- htmx call from the per-container Actions dropdown: the page they're
  -- on (/machines/<name>/...) no longer exists post-destroy, so redirect
  -- the browser to /machines via HX-Redirect.
  if is_htmx(req) then
    local kind = result.ok and "ok" or "err"
    local msg  = result.ok and ("Deleted " .. name .. ".")
                           or ("Failed to delete " .. name .. ": " .. (result.error or "unknown"))
    return {
      status  = 200,
      headers = {
        ["HX-Redirect"]  = ("/machines?kind=%s&msg=%s"):format(kind, urlencode(msg)),
        ["Content-Type"] = "text/html; charset=utf-8",
      },
      body = "",
    }
  end

  -- Plain browser form post (no htmx): 303 redirect.
  if wants_redirect(req) then
    local kind = result.ok and "ok" or "err"
    local msg  = result.ok and ("Deleted " .. name .. ".")
                           or ("Failed to delete " .. name .. ": " .. (result.error or "unknown"))
    return {
      status  = 303,
      headers = { ["Location"] = ("/machines?kind=%s&msg=%s"):format(kind, urlencode(msg)) },
    }
  end
  return json_response(result.ok and 200 or 400, result)
end

-- ── Edit CPU + memory limits on an existing container ────────────────────
--
-- POST /api/machines/<name>/resources
--   form fields: cpu_cores (optional), memory_gb (optional)
--   Both empty → clears the persistent drop-in and live-resets to infinity.
--   Either set → writes the persistent drop-in AND applies live via
--                `systemctl set-property --runtime` (no restart).
--
-- Response: HTML fragment for the Resources card (htmx swaps it inline).

local resources_svc = require("services.nspawn.resources")
local render_mod    = require("pages.render")

local function resources_card_response(req, name, current, err)
  current = current or resources_svc.read(name)
  local frag = render_mod.fragment("machine_resources_card", {
    machine_name = name,
    resources    = current,
    error        = err,
  })
  return {
    status = 200,
    headers = {
      ["HX-Trigger"]   = "dashboard-refresh",
      ["Content-Type"] = "text/html; charset=utf-8",
    },
    body = frag.body or frag,
  }
end

function M.resources(req)
  local name = (req.path or ""):match("^/api/machines/([^/]+)/resources$")
  if not name or not name:match("^[A-Za-z0-9._%-]+$") then
    return json_response(400, { ok = false, error = "invalid name" })
  end

  local body = form.parse(req)
  local cpu_cores, cpu_err = parse_optional_positive_number(body.cpu_cores, 256)
  if cpu_err then
    return resources_card_response(req, name, nil, "cpu_cores: " .. cpu_err)
  end
  local memory_gb, mem_err = parse_optional_positive_number(body.memory_gb, 1024)
  if mem_err then
    return resources_card_response(req, name, nil, "memory_gb: " .. mem_err)
  end

  local result = resources_svc.apply(name, cpu_cores, memory_gb)

  audit.append({
    actor  = actor(req),
    ip     = ip_of(req),
    action = "machine.resources." .. (result.ok and "set" or "set_failed"),
    target = name,
    result = result.ok and "ok" or "fail",
    reason = result.ok and nil or result.error,
    meta   = {
      cpu_cores = cpu_cores, memory_gb = memory_gb,
      live = result.live, applied = result.applied,
    },
  })

  if not result.ok then
    return resources_card_response(req, name, nil, result.error)
  end

  -- Read back current state for the swap — confirms the write landed.
  local current = resources_svc.read(name)
  local note
  if not result.live then
    note = result.note or
      "Persistent drop-in saved; container not running, will apply on next start."
  end
  return resources_card_response(req, name, current, note)
end

return M
