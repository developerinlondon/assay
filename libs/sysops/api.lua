--! Sysops api-handler registry.
--!
--! The predecessor monolith collected packages-related API handlers
--! here; those moved to plan 20's `pkg` stdlib. The sysops api/* tree
--! now contains only the host-ops surfaces, accessed directly via
--! `pages.lua` (events, machines, logs, audit, shell). This file is
--! retained as an intentional stub in case future sysops api-only
--! handlers (no page wrapper) need a registration point.

return {}
