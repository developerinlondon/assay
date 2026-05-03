--! Hostops page-handler registry.
--!
--! Returns a flat slug → handler table that mount.lua maps to URL
--! patterns. Each handler is a function `(req) → { status, body, ... }`.
--!
--! The predecessor monolith (knowhere0426) used this layer to also
--! collect routes for auth/vault/workflow/engine/zanzibar consoles.
--! Those have moved out of hostops; only host-ops surfaces remain.

local dash       = require("pages.dashboard")
local machine    = require("pages.machine")
local events     = require("api.events")
local machines   = require("api.machines")
local audit_pg   = require("pages.audit")
local audit_api  = require("api.audit")
local logs_pg    = require("pages.logs")
local logs_api   = require("api.logs")
local svcs_pg    = require("pages.services")
local cron_pg    = require("pages.cron")
local tunnels_pg = require("pages.tunnels")
local iface_pg   = require("pages.interfaces")
local tail_pg    = require("pages.tailscale")
local shell_pg   = require("pages.shell")
local shell_api  = require("api.shell")

local backups_pg          = require("pages.backups.index")
local backups_setup_pg    = require("pages.backups.setup")
local backups_sources_pg  = require("pages.backups.sources")
local backups_schedule_pg = require("pages.backups.schedule")
local backups_run_pg      = require("pages.backups.run")
local backups_restore_pg  = require("pages.backups.restore")
local backups_job_pg      = require("pages.backups.job")

local machines_index_pg = require("pages.machines.index")
local machines_new_pg   = require("pages.machines.new")
local m_services_pg     = require("pages.machines.services")
local m_cron_pg         = require("pages.machines.cron")
local m_logs_pg         = require("pages.machines.logs")

return {
  -- Top-level dashboard + SSE.
  dashboard      = dash.dashboard,
  events         = events.events,

  -- Dashboard fragments.
  host_strip      = dash.host_strip,
  machines_grid   = dash.machines_grid,
  status_strip    = dash.status_strip,
  recent_activity = dash.recent_activity,

  -- Machine detail page + fragments.
  machine_detail      = machine.detail,
  machine_utilization = machine.utilization,
  machine_processes   = machine.processes,
  machine_journal     = machine.journal,

  -- Per-container tabs.
  machine_services = m_services_pg.page,
  machine_cron     = m_cron_pg.page,
  machine_logs     = m_logs_pg.page,

  -- Machine lifecycle (POST).
  machine_action     = machines.handle,
  machine_provision  = machines.provision,
  machine_destroy    = machines.destroy,
  machine_job_status = machines.job_status,

  -- Browser shell (xterm.js + WS PTY bridge).
  shell_machine    = shell_pg.machine_page,
  shell_machine_ws = shell_api.handle_machine,
  shell_host       = shell_pg.host_page,
  shell_host_ws    = shell_api.handle_host,

  -- Host-level read-only pages.
  services    = svcs_pg.page,
  cron        = cron_pg.page,
  logs        = logs_pg.page,
  logs_stream = logs_api.stream,
  tunnels     = tunnels_pg.page,
  interfaces  = iface_pg.page,
  tailscale   = tail_pg.page,

  -- Backups (read-only views + setup actions).
  backups                 = backups_pg.page,
  backups_setup_test      = backups_setup_pg.test,
  backups_setup_init      = backups_setup_pg.init,
  backups_reconfigure     = backups_setup_pg.reconfigure,
  backups_sources_editor  = backups_sources_pg.editor,
  backups_sources_update  = backups_sources_pg.update,
  backups_schedule_editor = backups_schedule_pg.editor,
  backups_schedule_update = backups_schedule_pg.update,
  backups_run_now         = backups_run_pg.run,
  backups_snapshot_detail = backups_restore_pg.detail,
  backups_restore_action  = backups_restore_pg.restore,
  backups_job_detail      = backups_job_pg.detail,
  backups_job_status      = backups_job_pg.status,

  -- Audit log viewer + export.
  audit        = audit_pg.audit,
  audit_export = audit_api.export,

  -- nspawn provisioning (machines list + new-machine form).
  machines_index = machines_index_pg,
  provision_new  = machines_new_pg.page,
}
