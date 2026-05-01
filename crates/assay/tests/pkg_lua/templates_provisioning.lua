-- Tests for the [template.rootfs] / [template.nspawn] / [template.systemd]
-- subtables introduced for nspawn provisioning. Validates the schema gate
-- without exercising any privileged operation.
local pkg = require("assay.pkg")

local function check(cond, msg)
  if not cond then error(msg, 2) end
end

local catalog_dir = "tests/fixtures/pkg_catalog"
local fixtures    = "tests/fixtures/pkg_templates_provisioning"

local cat = pkg.catalog.load({ catalog_dir })
local tpl = pkg.templates.load({ fixtures }, cat.entries)

-- Helper to find a specific error among the loader's reported errors.
local function err_for(template_id, field_substr)
  for _, e in ipairs(tpl.errors) do
    if e.template_id == template_id
       and (not field_substr or (e.field and e.field:find(field_substr, 1, true))) then
      return e
    end
  end
  return nil
end

-- ── full valid template with all sections ─────────────────────────────────
local full = tpl.entries["full"]
check(full, "full provisioning template should load")
check(full.rootfs and full.rootfs.source == "machinectl-pull-tar",
      "rootfs section preserved with source")
check(full.rootfs.url:find("hub.nspawn.org", 1, true), "rootfs url preserved")
check(full.nspawn and full.nspawn.resolv_conf == "bind-host",
      "nspawn.resolv_conf preserved (hyphen form accepted)")
check(full.nspawn.boot == true, "nspawn.boot preserved")
check(type(full.nspawn.binds) == "table" and full.nspawn.binds[1] == "/dev/kmsg",
      "nspawn.binds preserved")
check(full.systemd and type(full.systemd.enable) == "table",
      "systemd.enable preserved")

-- ── packages-only template (no provisioning sections) ─────────────────────
local pkgsonly = tpl.entries["packages-only"]
check(pkgsonly, "packages-only template should still load (provisioning sections optional)")
check(pkgsonly.rootfs == nil and pkgsonly.nspawn == nil,
      "packages-only carries no provisioning sections")

-- ── snapshot-source template ──────────────────────────────────────────────
local snap = tpl.entries["snapshot-template"]
check(snap, "snapshot-template should load")
check(snap.rootfs.source == "machinectl-clone" and snap.rootfs.from == "_golden",
      "machinectl-clone source carries `from`")

-- ── invalid: unknown rootfs source ────────────────────────────────────────
check(tpl.entries["bad-rootfs-source"] == nil,
      "template with unknown rootfs.source should be rejected")
check(err_for("bad-rootfs-source", "rootfs.source"),
      "expected error pointing at rootfs.source")

-- ── invalid: pull-tar missing url ─────────────────────────────────────────
check(tpl.entries["bad-rootfs-pulltar-no-url"] == nil,
      "pull-tar without url should be rejected")
check(err_for("bad-rootfs-pulltar-no-url", "rootfs.url"),
      "expected error pointing at rootfs.url")

-- ── invalid: clone missing from ───────────────────────────────────────────
check(tpl.entries["bad-rootfs-clone-no-from"] == nil,
      "clone without from should be rejected")
check(err_for("bad-rootfs-clone-no-from", "rootfs.from"),
      "expected error pointing at rootfs.from")

-- ── invalid: bogus resolv_conf value ──────────────────────────────────────
check(tpl.entries["bad-resolv-conf"] == nil,
      "template with bogus nspawn.resolv_conf should be rejected")
check(err_for("bad-resolv-conf", "resolv_conf"),
      "expected error pointing at nspawn.resolv_conf")

-- ── invalid: nspawn.boot is a string instead of boolean ───────────────────
check(tpl.entries["bad-nspawn-types"] == nil,
      "template with non-boolean nspawn.boot should be rejected")
check(err_for("bad-nspawn-types", "nspawn.boot"),
      "expected error pointing at nspawn.boot type")

-- ── invalid: systemd.enable is a string instead of array ──────────────────
check(tpl.entries["bad-systemd-enable"] == nil,
      "template with non-array systemd.enable should be rejected")
check(err_for("bad-systemd-enable", "systemd.enable"),
      "expected error pointing at systemd.enable type")

-- ── valid: hyphen form of resolv_conf normalizes ──────────────────────────
local hy = tpl.entries["resolv-conf-hyphen"]
check(hy and hy.nspawn.resolv_conf == "bind-host",
      "hyphen form of resolv_conf accepted (matches systemd-nspawn flag spelling)")

print("templates_provisioning.lua OK")
