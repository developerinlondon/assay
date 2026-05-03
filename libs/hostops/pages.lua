local stubs     = require("pages.stubs")
local dash      = require("pages.dashboard")
local machine   = require("pages.machine")
local events    = require("api.events")
local machines  = require("api.machines")
local audit_pg  = require("pages.audit")
local audit_api = require("api.audit")
local logs_pg   = require("pages.logs")
local logs_api  = require("api.logs")
local svcs_pg   = require("pages.services")
local cron_pg   = require("pages.cron")
local tunnels_pg   = require("pages.tunnels")
local iface_pg     = require("pages.interfaces")
local tailscale_pg = require("pages.tailscale")
local backups_pg          = require("pages.backups.index")
local backups_setup_pg    = require("pages.backups.setup")
local backups_sources_pg  = require("pages.backups.sources")
local backups_schedule_pg = require("pages.backups.schedule")
local backups_run_pg      = require("pages.backups.run")
local backups_restore_pg  = require("pages.backups.restore")
local backups_job_pg      = require("pages.backups.job")
local packages_pg        = require("pages.packages")
local packages_target_pg = require("pages.packages.target")
local packages_detail_pg = require("pages.packages.detail")
local shell_pg     = require("pages.shell")
local shell_api    = require("api.shell")
local pkg_api      = require("api.packages")

-- v0.2.0 — native engine pages (plan 09 follow-up).
-- Each module returns the handler function directly.
local engine_info_pg      = require("pages.engine.index")
local engine_modules_pg   = require("pages.engine.modules")
local engine_instances_pg = require("pages.engine.instances")
local engine_audit_pg     = require("pages.engine.audit")
local engine_config_pg    = require("pages.engine.config")

local wf_index_pg      = require("pages.workflows.index")
local wf_run_pg        = require("pages.workflows.run")
local wf_schedules_pg  = require("pages.workflows.schedules")
local wf_namespaces_pg = require("pages.workflows.namespaces")
local wf_workers_pg    = require("pages.workflows.workers")
local wf_queues_pg     = require("pages.workflows.queues")
local wf_settings_pg   = require("pages.workflows.settings")

local auth_users_pg        = require("pages.auth.users")
local auth_sessions_pg     = require("pages.auth.sessions")
local auth_oidc_clients_pg = require("pages.auth.oidc_clients")
local auth_upstreams_pg    = require("pages.auth.upstreams")
local auth_jwks_pg         = require("pages.auth.jwks")
local auth_biscuit_pg      = require("pages.auth.biscuit")
local auth_audit_pg        = require("pages.auth.audit")
local auth_user_edit_pg    = require("pages.auth.user_edit")

local vault_index_pg       = require("pages.vault.index")
local vault_kv_pg          = require("pages.vault.kv")
local vault_transit_pg     = require("pages.vault.transit")
local vault_sealing_pg     = require("pages.vault.sealing")
local vault_dynamic_pg     = require("pages.vault.dynamic")
local vault_me_pg          = require("pages.vault.me")
local vault_collections_pg = require("pages.vault.collections")
local vault_share_pg       = require("pages.vault.share")

local zb_index_pg  = require("pages.zanzibar.index")
local zb_tuples_pg = require("pages.zanzibar.tuples")
local zb_check_pg  = require("pages.zanzibar.check")

local plugins_index_pg  = require("pages.plugins.index")
local machines_index_pg = require("pages.machines.index")
local machines_new_pg   = require("pages.machines.new")
local m_services_pg     = require("pages.machines.services")
local m_cron_pg         = require("pages.machines.cron")
local m_logs_pg         = require("pages.machines.logs")

-- POST handlers (mutation endpoints). Kept in separate modules so the
-- read-only page handlers stay focused on rendering.
local engine_actions   = require("pages.engine.actions")
local zanzibar_actions = require("pages.zanzibar.actions")
local auth_actions     = require("pages.auth.actions")
local vault_actions    = require("pages.vault.actions")
local workflow_actions = require("pages.workflows.actions")

return {
  dashboard      = dash.dashboard,
  machine_detail = machine.detail,

  -- SSE
  events         = events.events,

  -- Overview fragments
  host_strip      = dash.host_strip,
  machines_grid   = dash.machines_grid,
  status_strip    = dash.status_strip,
  recent_activity = dash.recent_activity,

  -- Machine-detail fragments
  machine_utilization = machine.utilization,
  machine_processes   = machine.processes,
  machine_journal     = machine.journal,

  -- Per-container tabs
  machine_services    = m_services_pg.page,
  machine_cron        = m_cron_pg.page,
  machine_logs        = m_logs_pg.page,

  -- Lifecycle POST handler
  machine_action     = machines.handle,
  machine_provision  = machines.provision,
  machine_destroy    = machines.destroy,
  machine_job_status = machines.job_status,

  -- Browser shell (xterm.js + WS PTY bridge)
  shell_machine    = shell_pg.machine_page,
  shell_machine_ws = shell_api.handle_machine,
  shell_host       = shell_pg.host_page,
  shell_host_ws    = shell_api.handle_host,

  services      = svcs_pg.page,
  cron          = cron_pg.page,
  logs          = logs_pg.page,
  logs_stream   = logs_api.stream,
  tunnels       = tunnels_pg.page,
  interfaces    = iface_pg.page,
  tailscale     = tailscale_pg.page,
  inventory     = stubs.inventory,
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
  audit         = audit_pg.audit,
  audit_export  = audit_api.export,
  packages        = packages_pg.page,
  packages_target = packages_target_pg.page,
  packages_detail  = packages_detail_pg.page,
  packages_catalog       = pkg_api.catalog,
  packages_templates     = pkg_api.templates,
  packages_state         = function(req)
    if req.method == "POST" then return pkg_api.mutate_state(req) else return pkg_api.get_state(req) end
  end,
  packages_reconcile     = pkg_api.reconcile,
  packages_check_updates = pkg_api.check_updates,
  packages_update_all    = pkg_api.update_all,
  packages_job_status    = pkg_api.job_status,
  settings      = stubs.settings,
  provision_new = machines_new_pg.page,

  -- v0.2.0 — native engine pages (read-only). Each handler is
  -- the module value itself (return function(req)...end).
  engine_info      = engine_info_pg,
  engine_modules   = engine_modules_pg,
  engine_instances = engine_instances_pg,
  engine_audit     = engine_audit_pg,
  engine_config    = engine_config_pg,

  workflow_runs       = wf_index_pg,
  workflow_run        = wf_run_pg,
  workflow_schedules  = wf_schedules_pg,
  workflow_namespaces = wf_namespaces_pg,
  workflow_workers    = wf_workers_pg,
  workflow_queues     = wf_queues_pg,
  workflow_settings   = wf_settings_pg,

  auth_users        = auth_users_pg,
  auth_sessions     = auth_sessions_pg,
  auth_oidc_clients = auth_oidc_clients_pg,
  auth_upstreams    = auth_upstreams_pg,
  auth_jwks         = auth_jwks_pg,
  auth_biscuit      = auth_biscuit_pg,
  auth_audit        = auth_audit_pg,
  auth_user_edit    = auth_user_edit_pg,

  vault_index       = vault_index_pg,
  vault_kv          = vault_kv_pg,
  vault_transit     = vault_transit_pg,
  vault_sealing     = vault_sealing_pg,
  vault_dynamic     = vault_dynamic_pg,
  vault_me          = vault_me_pg,
  vault_collections = vault_collections_pg,
  vault_share       = vault_share_pg,

  zanzibar_index  = zb_index_pg,
  zanzibar_tuples = zb_tuples_pg,
  zanzibar_check  = zb_check_pg,

  plugins_index  = plugins_index_pg,
  machines_index = machines_index_pg,

  -- POST handlers
  engine_modules_action  = engine_actions.modules_dispatch,
  zanzibar_write_tuple   = zanzibar_actions.write_tuple,
  zanzibar_delete_tuple  = zanzibar_actions.delete_tuple,
  zanzibar_run_check     = zanzibar_actions.run_check,

  auth_users_action          = auth_actions.users_dispatch,
  auth_sessions_action       = auth_actions.sessions_dispatch,
  auth_oidc_clients_action   = auth_actions.oidc_clients_dispatch,
  auth_upstreams_action      = auth_actions.upstreams_dispatch,

  -- Vault write actions (sealing/kv/transit/share). Per-resource
  -- dispatchers route based on req.path; main.lua mounts each on a
  -- /vault/<resource>/* wildcard.
  vault_sealing_action       = vault_actions.sealing_dispatch,
  vault_kv_action            = vault_actions.kv_dispatch,
  vault_transit_action       = vault_actions.transit_dispatch,
  vault_share_action         = vault_actions.share_dispatch,

  -- Workflow write actions (run/schedule/namespace dispatchers).
  workflow_runs_action       = workflow_actions.runs_dispatch,
  workflow_schedules_action  = workflow_actions.schedules_dispatch,
  workflow_namespaces_action = workflow_actions.namespaces_dispatch,
  workflow_start_action      = workflow_actions.start_workflow,
}
