--- assay.workflow — client library for the assay workflow engine.
---
--- An assay Lua app that calls `workflow.listen()` becomes a worker for a
--- running `assay serve` instance. Workflows are written as plain Lua
--- functions; activities are also plain Lua functions. The engine drives
--- execution via two task types polled by the same worker:
---
---  - **Workflow tasks**: orchestration. The handler is re-run from scratch
---    on every "step" — each `ctx:execute_activity / sleep / wait_for_signal`
---    call either returns a value cached in the workflow's event history
---    (deterministic replay) or yields a command for the engine to schedule.
---  - **Activity tasks**: concrete work. The handler runs once; its return
---    value is persisted as `ActivityCompleted` so future replays of the
---    parent workflow short-circuit at that step.
---
--- The worker survives engine restarts, network blips, and worker crashes:
--- on resume any other worker on the queue can pick up the workflow task,
--- replay from the event log, and reach the same point. Side effects are
--- never executed twice as long as workflow code is deterministic — see
--- `ctx:side_effect` for the escape hatch when it isn't.
---
--- Module layout (v0.12+): the public surface lives here; the bulk lives
--- in sibling submodules so each file stays under ~500 lines and agents /
--- humans can navigate without scrolling endlessly.
---
---   stdlib/workflow.lua          — public API + glue (this file)
---   stdlib/workflow/api.lua      — HTTP wrapper used by every method
---   stdlib/workflow/listen.lua   — worker registration + poll loop
---   stdlib/workflow/task.lua     — workflow + activity task handlers
---   stdlib/workflow/ctx.lua      — workflow ctx factory (replay machinery)
---
--- Submodule functions take this module table (M) as their first arg so
--- shared state (`M._workflows`, `M._activities`, `M._engine_url`, etc.)
--- stays on the parent without circular requires.
---
--- Usage:
---   local workflow = require("assay.workflow")
---   workflow.connect("http://assay-server:8080")
---
---   workflow.define("IngestData", function(ctx, input)
---       local data = ctx:execute_activity("fetch_s3", { bucket = input.source })
---       ctx:sleep(10)
---       ctx:execute_activity("load_warehouse", { data = data })
---       return { status = "done" }
---   end)
---
---   workflow.activity("fetch_s3", function(ctx, input)
---       return http.get("https://s3/" .. input.bucket).body
---   end)
---
---   workflow.listen({ queue = "data-pipeline" })

local api_mod = require("assay.workflow.api")
local listen_mod = require("assay.workflow.listen")
local task_mod = require("assay.workflow.task")
local ctx_mod = require("assay.workflow.ctx")

local M = {}

-- Shared state, hung off M so submodules can read/write via the parent
-- reference passed as their first arg. Underscore-prefixed to mark
-- private (not part of the public API).
M._engine_url = nil
M._workflows = {}
M._activities = {}
M._worker_id = nil
M._auth_token = nil

-- ── Submodule glue ──────────────────────────────────────────
-- These wrappers exist for backward compatibility — code that called
-- `workflow._api(...)`, `workflow._make_workflow_ctx(...)`, etc.
-- pre-split keeps working unchanged.
function M._api(method, path, body) return api_mod.call(M, method, path, body) end
function M._make_workflow_ctx(workflow_id, history)
    return ctx_mod.make(M, workflow_id, history)
end
function M._handle_workflow_task(task) return task_mod.handle_workflow_task(M, task) end
function M._collect_snapshot(ctx) return task_mod.collect_snapshot(ctx) end
function M._execute_activity(task) return task_mod.execute_activity(M, task) end
function M._make_activity_ctx(task) return task_mod.make_activity_ctx(M, task) end
function M._poll_workflow_task(queue) return listen_mod.poll_workflow_task(M, queue) end
function M._poll_activity_task(queue) return listen_mod.poll_activity_task(M, queue) end

--- Minimal URL-encoder for query params. Covers the characters we actually
--- emit from this stdlib: namespace names, schedule names, JSON filter
--- payloads. Not a full RFC 3986 encoder.
local function url_encode(s)
    return (tostring(s):gsub("([^A-Za-z0-9%-_.~])", function(c)
        return string.format("%%%02X", string.byte(c))
    end))
end

--- Connect to the workflow engine.
--- @param url string Engine URL (e.g. "http://localhost:8080")
--- @param opts? table Optional: { token = "Bearer ..." }
function M.connect(url, opts)
    M._engine_url = url:gsub("/$", "") -- strip trailing slash
    if opts and opts.token then
        M._auth_token = opts.token
    end
    -- Verify connectivity
    local resp = M._api("GET", "/health")
    if resp.status ~= 200 then
        error("workflow.connect: cannot reach engine at " .. url)
    end
    log.info("Connected to workflow engine at " .. url)
end

--- Define a workflow type. The handler receives a `ctx` whose methods
--- (`execute_activity`, `sleep`, `wait_for_signal`, `side_effect`) drive
--- the engine — see the module-level docstring for the replay model.
--- @param name string Workflow type name (matches `workflow_type` on start)
--- @param handler function(ctx, input) -> result
function M.define(name, handler)
    M._workflows[name] = handler
end

--- Define an activity implementation.
--- @param name string Activity name
--- @param handler function(ctx, input) -> result
function M.activity(name, handler)
    M._activities[name] = handler
end

--- Start a workflow on the engine (client-side, not as a worker).
--- @param opts table { workflow_type, workflow_id, namespace?, input?, task_queue?, search_attributes? }
--- @return table { workflow_id, run_id, status }
function M.start(opts)
    local body = {
        workflow_type = opts.workflow_type,
        workflow_id = opts.workflow_id,
        -- `namespace` is optional; the engine defaults to "main" when
        -- absent. Passing it lets callers scope a run to a non-default
        -- namespace (e.g. "deployments", "platform") so listing,
        -- schedules, and worker registration stay partitioned.
        namespace = opts.namespace,
        input = opts.input,
        task_queue = opts.task_queue or "default",
        search_attributes = opts.search_attributes,
    }
    local resp = M._api("POST", "/workflows", body)
    if resp.status ~= 201 then
        error("workflow.start failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Send a signal to a running workflow.
function M.signal(workflow_id, signal_name, payload)
    local body = payload and { payload = payload } or {}
    local resp = M._api("POST", "/workflows/" .. workflow_id .. "/signal/" .. signal_name, body)
    if resp.status ~= 200 then
        error("workflow.signal failed: " .. (resp.body or "unknown error"))
    end
end

--- Query a workflow's current state.
function M.describe(workflow_id)
    local resp = M._api("GET", "/workflows/" .. workflow_id)
    if resp.status ~= 200 then
        error("workflow.describe failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Cancel a running workflow.
function M.cancel(workflow_id)
    local resp = M._api("POST", "/workflows/" .. workflow_id .. "/cancel")
    if resp.status ~= 200 then
        error("workflow.cancel failed: " .. (resp.body or "unknown error"))
    end
end

--- Terminate a workflow immediately with a reason (harder than cancel: no
--- graceful handler cleanup, workflow goes straight to FAILED).
function M.terminate(workflow_id, reason)
    local body = reason and { reason = reason } or {}
    local resp = M._api("POST", "/workflows/" .. workflow_id .. "/terminate", body)
    if resp.status ~= 200 then
        error("workflow.terminate failed: " .. (resp.body or "unknown error"))
    end
end

--- List workflows in a namespace with optional filters.
--- @param opts? table { namespace?, status?, type?, search_attrs?, limit?, offset? }
--- @return table array of workflow records
function M.list(opts)
    opts = opts or {}
    local params = {}
    if opts.namespace then params[#params + 1] = "namespace=" .. url_encode(opts.namespace) end
    if opts.status then params[#params + 1] = "status=" .. url_encode(opts.status) end
    if opts.type then params[#params + 1] = "type=" .. url_encode(opts.type) end
    if opts.search_attrs then
        params[#params + 1] = "search_attrs="
            .. url_encode(json.encode(opts.search_attrs))
    end
    if opts.limit then params[#params + 1] = "limit=" .. tostring(opts.limit) end
    if opts.offset then params[#params + 1] = "offset=" .. tostring(opts.offset) end
    local path = "/workflows"
    if #params > 0 then path = path .. "?" .. table.concat(params, "&") end
    local resp = M._api("GET", path)
    if resp.status ~= 200 then
        error("workflow.list failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Fetch the full event history of a workflow.
function M.get_events(workflow_id)
    local resp = M._api("GET", "/workflows/" .. workflow_id .. "/events")
    if resp.status ~= 200 then
        error("workflow.get_events failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Read the latest snapshot written by `ctx:register_query` handlers.
--- With a query name, returns just that query's value. Without, returns
--- the full snapshot table.
--- @param workflow_id string
--- @param name? string Specific query handler name
--- @return table|any state or single value
function M.get_state(workflow_id, name)
    local path = "/workflows/" .. workflow_id .. "/state"
    if name then path = path .. "/" .. name end
    local resp = M._api("GET", path)
    if resp.status == 404 then
        return nil -- no snapshot recorded yet (or unknown query name)
    end
    if resp.status ~= 200 then
        error("workflow.get_state failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- List a parent workflow's child workflows.
function M.list_children(workflow_id)
    local resp = M._api("GET", "/workflows/" .. workflow_id .. "/children")
    if resp.status ~= 200 then
        error("workflow.list_children failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Close out `workflow_id` and start a fresh run with the same type,
--- namespace, and task queue. This is the *client-side* variant of
--- continue-as-new: called from outside the workflow handler. The
--- worker-side `ctx:continue_as_new(input)` does the same thing from
--- inside a workflow handler.
--- @param workflow_id string The workflow to close out
--- @param input? any JSON-encodable input for the new run
--- @return table workflow record for the new run
function M.continue_as_new(workflow_id, input)
    local body = { input = input }
    local resp = M._api(
        "POST",
        "/workflows/" .. workflow_id .. "/continue-as-new",
        body
    )
    if resp.status ~= 201 then
        error("workflow.continue_as_new failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

-- ── Schedules ───────────────────────────────────────────────
--- Schedule CRUD + lifecycle. Each call returns the schedule record
--- (create/describe/patch/pause/resume) or raises on HTTP error.

M.schedules = {}

--- Create a new cron schedule.
--- @param opts table {
---   name, workflow_type, cron_expr, timezone?, input?,
---   task_queue?, overlap_policy?, namespace?
--- }
function M.schedules.create(opts)
    if not opts or not opts.name or not opts.workflow_type or not opts.cron_expr then
        error("workflow.schedules.create: name, workflow_type, cron_expr required")
    end
    local resp = M._api("POST", "/schedules", opts)
    if resp.status ~= 201 then
        error("workflow.schedules.create failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- List schedules in a namespace.
--- @param opts? table { namespace? }
function M.schedules.list(opts)
    local ns = (opts and opts.namespace) or "main"
    local resp = M._api("GET", "/schedules?namespace=" .. url_encode(ns))
    if resp.status ~= 200 then
        error("workflow.schedules.list failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Describe one schedule.
function M.schedules.describe(name, opts)
    local ns = (opts and opts.namespace) or "main"
    local resp = M._api(
        "GET",
        "/schedules/" .. url_encode(name) .. "?namespace=" .. url_encode(ns)
    )
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
        error("workflow.schedules.describe failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Apply an in-place patch to a schedule. Only fields present in the
--- patch are updated; unchanged fields keep their values.
--- @param name string
--- @param patch table { cron_expr?, timezone?, input?, task_queue?, overlap_policy? }
--- @param opts? table { namespace? }
function M.schedules.patch(name, patch, opts)
    local ns = (opts and opts.namespace) or "main"
    local resp = M._api(
        "PATCH",
        "/schedules/" .. url_encode(name) .. "?namespace=" .. url_encode(ns),
        patch or {}
    )
    if resp.status ~= 200 then
        error("workflow.schedules.patch failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Pause a schedule (the scheduler skips it until resumed).
function M.schedules.pause(name, opts)
    local ns = (opts and opts.namespace) or "main"
    local resp = M._api(
        "POST",
        "/schedules/" .. url_encode(name) .. "/pause?namespace=" .. url_encode(ns)
    )
    if resp.status ~= 200 then
        error("workflow.schedules.pause failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Resume a paused schedule. Does not backfill missed fires — next fire
--- is whatever the cron expression says from now.
function M.schedules.resume(name, opts)
    local ns = (opts and opts.namespace) or "main"
    local resp = M._api(
        "POST",
        "/schedules/" .. url_encode(name) .. "/resume?namespace=" .. url_encode(ns)
    )
    if resp.status ~= 200 then
        error("workflow.schedules.resume failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Delete a schedule. Any in-flight workflow it triggered keeps running
--- — deletion only stops future fires.
function M.schedules.delete(name, opts)
    local ns = (opts and opts.namespace) or "main"
    local resp = M._api(
        "DELETE",
        "/schedules/" .. url_encode(name) .. "?namespace=" .. url_encode(ns)
    )
    if resp.status ~= 200 then
        error("workflow.schedules.delete failed: " .. (resp.body or "unknown error"))
    end
end

-- ── Namespaces ──────────────────────────────────────────────

M.namespaces = {}

--- Create a namespace. The endpoint returns no body on success; returns
--- nothing from Lua on success, raises on error.
function M.namespaces.create(name)
    local resp = M._api("POST", "/namespaces", { name = name })
    if resp.status ~= 201 and resp.status ~= 200 then
        error("workflow.namespaces.create failed: " .. (resp.body or "unknown error"))
    end
end

--- List namespaces.
function M.namespaces.list()
    local resp = M._api("GET", "/namespaces")
    if resp.status ~= 200 then
        error("workflow.namespaces.list failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Get per-namespace stats (total, running, pending, completed, failed,
--- schedules, workers). Same endpoint as describe — the stats are the
--- response body.
function M.namespaces.stats(name)
    local resp = M._api("GET", "/namespaces/" .. url_encode(name))
    if resp.status ~= 200 then
        error("workflow.namespaces.stats failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Alias of `stats` for symmetry with schedules.describe etc.
function M.namespaces.describe(name)
    return M.namespaces.stats(name)
end

--- Delete a namespace.
function M.namespaces.delete(name)
    local resp = M._api("DELETE", "/namespaces/" .. url_encode(name))
    if resp.status ~= 200 then
        error("workflow.namespaces.delete failed: " .. (resp.body or "unknown error"))
    end
end

-- ── API Keys ────────────────────────────────────────────────

M.api_keys = {}

--- Generate (or idempotently retrieve) an API key.
---
--- @param label string|nil  Optional label to tag the key with.
--- @param opts table|nil    `{ idempotent = bool }`. When true and a key
---                          with this `label` already exists, the server
---                          returns the existing record's metadata without
---                          a plaintext (which is only ever retrievable at
---                          generation time).
--- @return table            `{ plaintext?, prefix, label, created_at }`.
---                          `plaintext` is present only on a fresh mint.
---
--- Bootstrap window: this call works without authentication iff the
--- server's `api_keys` table is empty (first-ever key). After that, a
--- valid Bearer token is required.
function M.api_keys.generate(label, opts)
    opts = opts or {}
    local body = { label = label }
    if opts.idempotent ~= nil then
        body.idempotent = opts.idempotent
    end
    local resp = M._api("POST", "/api-keys", body)
    if resp.status ~= 200 and resp.status ~= 201 then
        error("workflow.api_keys.generate failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- List API key metadata (hashes never exposed). Returns prefix + label
--- + created_at per key.
function M.api_keys.list()
    local resp = M._api("GET", "/api-keys")
    if resp.status ~= 200 then
        error("workflow.api_keys.list failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Revoke an API key by its prefix (e.g. "assay_abcd1234...").
function M.api_keys.delete(prefix)
    local resp = M._api("DELETE", "/api-keys/" .. url_encode(prefix))
    if resp.status ~= 204 and resp.status ~= 404 then
        error("workflow.api_keys.delete failed: " .. (resp.body or "unknown error"))
    end
end

-- ── Workers ─────────────────────────────────────────────────

M.workers = {}

--- List registered workers (and their last heartbeat).
--- @param opts? table { namespace? }
function M.workers.list(opts)
    local ns = (opts and opts.namespace) or "main"
    local resp = M._api("GET", "/workers?namespace=" .. url_encode(ns))
    if resp.status ~= 200 then
        error("workflow.workers.list failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

-- ── Queues ──────────────────────────────────────────────────

M.queues = {}

--- Pending/running activity counts per task queue.
--- @param opts? table { namespace? }
function M.queues.stats(opts)
    local ns = (opts and opts.namespace) or "main"
    local resp = M._api("GET", "/queues?namespace=" .. url_encode(ns))
    if resp.status ~= 200 then
        error("workflow.queues.stats failed: " .. (resp.body or "unknown error"))
    end
    return json.parse(resp.body)
end

--- Start listening for tasks. Blocks until cancelled. See
--- `stdlib/workflow/listen.lua` for the loop body.
--- @param opts table { identity?, queue?, namespace?, max_concurrent_workflows?, max_concurrent_activities? }
function M.listen(opts)
    return listen_mod.listen(M, opts)
end

return M
