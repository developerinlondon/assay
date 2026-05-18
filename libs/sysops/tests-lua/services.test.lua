--! sysops service-unit helper tests.
--!
--! Run via:
--!   LUA_PATH='libs/?.lua;libs/?/init.lua;libs/sysops/?.lua;libs/sysops/tests-lua/?.lua;;' \
--!     assay libs/sysops/tests-lua/services.test.lua

print("[sysops.services]")

local services = require("services.host.service_units")

local original_systemd = systemd
local action_calls = {}

local function detail_value(row, label)
  for _, d in ipairs(row.details or {}) do
    if d.label == label then return d.value end
  end
  return nil
end

systemd = {
  unit_status = function(name)
    if name == "demo.service" then
      return {
        memory_current = 67108864,
        tasks_current = 22,
        cpu_usage_nsec = 630000000,
        n_restarts = 2,
        unit_file_state = "enabled",
        fragment_path = "/etc/systemd/system/demo.service",
        main_pid = 4242,
        exec_start = "/usr/bin/demo --flag",
        restart = "on-failure",
        user = "demo",
      }
    end
    return {}
  end,
  unit_action = function(unit, action, opts)
    action_calls[#action_calls + 1] = {
      unit = unit,
      action = action,
      timeout = opts and opts.timeout,
    }
    return { status = 0, stdout = "", stderr = "" }
  end,
}

do
  local t = 100
  local calls = 0
  local samples = {
    {
      ["demo.service"] = 1000000000,
      ["idle.service"] = 500000000,
    },
    {
      ["demo.service"] = 1200000000,
      ["idle.service"] = 500000000,
    },
  }
  local usage = services.sample_cpu_usage({
    { name = "demo.service" },
    { name = "idle.service" },
    { name = "demo.timer" },
  }, {
    interval = 0.5,
    clock = function() return t end,
    sleep = function(seconds) t = t + seconds end,
    sample = function()
      calls = calls + 1
      return samples[calls] or {}
    end,
  })

  assert.eq(calls, 2, "CPU usage samples twice")
  assert.eq(usage["demo.service"], 40, "CPU usage uses CPU delta over wall time")
  assert.eq(usage["idle.service"], 0, "CPU usage handles idle units")
  assert.eq(usage["demo.timer"], nil, "CPU usage skips non-service units")
end

do
  local rows = services.enrich({
    {
      name = "demo.service",
      load = "loaded",
      active = "active",
      sub = "running",
      description = "Demo service",
    },
    {
      name = "demo.timer",
      load = "loaded",
      active = "active",
      sub = "waiting",
      description = "Demo timer",
    },
  }, {
    cpu_usage = {
      ["demo.service"] = 40,
    },
  })

  assert.eq(rows[1].memory, "64 M", "service memory is human formatted")
  assert.eq(rows[1].memory_sort, 67108864, "service memory sort value")
  assert.eq(rows[1].tasks, 22, "service tasks are numeric")
  assert.eq(rows[1].tasks_label, "22", "service tasks label")
  assert.eq(rows[1].cpu_usage, 40, "service CPU usage value")
  assert.eq(rows[1].cpu_label, "40%", "service CPU usage is human formatted")
  assert.eq(rows[1].cpu_sort, 40, "service CPU sort value")
  assert.eq(rows[1].restarts, 2, "service restart count")
  assert.eq(rows[1].restart_allowed, true, "service restart allowed")
  assert.eq(rows[1].dom_id, "svc-demo-service", "service gets stable detail DOM id")
  assert.eq(detail_value(rows[1], "Unit file"), "enabled", "service detail includes unit file state")
  assert.eq(detail_value(rows[1], "Fragment path"), "/etc/systemd/system/demo.service", "service detail includes unit path")
  assert.eq(detail_value(rows[1], "Main PID"), "4242", "service detail includes main pid")
  assert.eq(detail_value(rows[1], "Exec start"), "/usr/bin/demo --flag", "service detail includes exec command")
  assert.eq(detail_value(rows[1], "Restart policy"), "on-failure", "service detail includes restart policy")

  assert.eq(rows[2].memory, "—", "non-service memory placeholder")
  assert.eq(rows[2].memory_sort, nil, "non-service memory sort placeholder")
  assert.eq(rows[2].cpu_label, "—", "non-service CPU usage placeholder")
  assert.eq(rows[2].cpu_sort, nil, "non-service CPU sort placeholder")
  assert.eq(rows[2].restart_allowed, false, "non-service restart blocked")
  assert.eq(#(rows[2].details or {}), 0, "non-service detail list is empty")
end

do
  local res = services.restart("demo.service")
  assert.eq(res.ok, true, "restart succeeds")
  assert.eq(#action_calls, 1, "restart calls unit_action once")
  assert.eq(action_calls[1].unit, "demo.service", "restart unit")
  assert.eq(action_calls[1].action, "restart", "restart action")
  assert.eq(action_calls[1].timeout, 60, "restart timeout")
end

do
  local res = services.start("demo.service")
  assert.eq(res.ok, true, "start succeeds")
  assert.eq(#action_calls, 2, "start calls unit_action once")
  assert.eq(action_calls[2].unit, "demo.service", "start unit")
  assert.eq(action_calls[2].action, "start", "start action")
  assert.eq(action_calls[2].timeout, 60, "start timeout")
end

do
  local res = services.stop("demo.service")
  assert.eq(res.ok, true, "stop succeeds")
  assert.eq(#action_calls, 3, "stop calls unit_action once")
  assert.eq(action_calls[3].unit, "demo.service", "stop unit")
  assert.eq(action_calls[3].action, "stop", "stop action")
  assert.eq(action_calls[3].timeout, 60, "stop timeout")
end

do
  local res = services.restart("demo.timer")
  assert.eq(res.ok, false, "restart rejects non-service unit")
  assert.contains(res.error, ".service", "restart rejection explains service-only rule")
end

do
  local res = services.action("demo.service", "reload")
  assert.eq(res.ok, false, "unsupported actions are rejected")
  assert.contains(res.error, "unsupported", "unsupported action explains rejection")
end

systemd = original_systemd

print("[sysops.services] all passed")
