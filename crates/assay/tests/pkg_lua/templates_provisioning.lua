-- Tests for the [template.rootfs] / [template.nspawn] / [template.systemd]
-- subtables introduced for nspawn provisioning. Validates the schema gate
-- without exercising any privileged operation.
local pkg = require("assay.pkg")

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
assert.not_nil(full, "full provisioning template should load")
assert.not_nil(full.rootfs, "rootfs section preserved")
assert.eq(full.rootfs.source, "machinectl-pull-tar", "rootfs section preserved with source")
assert.contains(full.rootfs.url, "hub.nspawn.org", "rootfs url preserved")
assert.not_nil(full.nspawn, "nspawn section preserved")
assert.eq(full.nspawn.resolv_conf, "bind-host",
  "nspawn.resolv_conf preserved (hyphen form accepted)")
assert.eq(full.nspawn.boot, true, "nspawn.boot preserved")
assert.eq(type(full.nspawn.binds), "table", "nspawn.binds preserved")
assert.eq(full.nspawn.binds[1], "/dev/kmsg", "nspawn.binds preserved")
assert.not_nil(full.systemd, "systemd section preserved")
assert.eq(type(full.systemd.enable), "table", "systemd.enable preserved")

-- ── packages-only template (no provisioning sections) ─────────────────────
local pkgsonly = tpl.entries["packages-only"]
assert.not_nil(pkgsonly, "packages-only template should still load (provisioning sections optional)")
assert.eq(pkgsonly.rootfs, nil, "packages-only carries no rootfs section")
assert.eq(pkgsonly.nspawn, nil, "packages-only carries no nspawn section")

-- ── snapshot-source template ──────────────────────────────────────────────
local snap = tpl.entries["snapshot-template"]
assert.not_nil(snap, "snapshot-template should load")
assert.eq(snap.rootfs.source, "machinectl-clone", "machinectl-clone source preserved")
assert.eq(snap.rootfs.from, "_golden", "machinectl-clone source carries `from`")

-- ── invalid: unknown rootfs source ────────────────────────────────────────
assert.eq(tpl.entries["bad-rootfs-source"], nil,
  "template with unknown rootfs.source should be rejected")
assert.not_nil(err_for("bad-rootfs-source", "rootfs.source"),
  "expected error pointing at rootfs.source")

-- ── invalid: pull-tar missing url ─────────────────────────────────────────
assert.eq(tpl.entries["bad-rootfs-pulltar-no-url"], nil,
  "pull-tar without url should be rejected")
assert.not_nil(err_for("bad-rootfs-pulltar-no-url", "rootfs.url"),
  "expected error pointing at rootfs.url")

-- ── invalid: clone missing from ───────────────────────────────────────────
assert.eq(tpl.entries["bad-rootfs-clone-no-from"], nil,
  "clone without from should be rejected")
assert.not_nil(err_for("bad-rootfs-clone-no-from", "rootfs.from"),
  "expected error pointing at rootfs.from")

-- ── invalid: bogus resolv_conf value ──────────────────────────────────────
assert.eq(tpl.entries["bad-resolv-conf"], nil,
  "template with bogus nspawn.resolv_conf should be rejected")
assert.not_nil(err_for("bad-resolv-conf", "resolv_conf"),
  "expected error pointing at nspawn.resolv_conf")

-- ── invalid: nspawn.boot is a string instead of boolean ───────────────────
assert.eq(tpl.entries["bad-nspawn-types"], nil,
  "template with non-boolean nspawn.boot should be rejected")
assert.not_nil(err_for("bad-nspawn-types", "nspawn.boot"),
  "expected error pointing at nspawn.boot type")

-- ── invalid: systemd.enable is a string instead of array ──────────────────
assert.eq(tpl.entries["bad-systemd-enable"], nil,
  "template with non-array systemd.enable should be rejected")
assert.not_nil(err_for("bad-systemd-enable", "systemd.enable"),
  "expected error pointing at systemd.enable type")

-- ── valid: hyphen form of resolv_conf normalizes ──────────────────────────
local hy = tpl.entries["resolv-conf-hyphen"]
assert.not_nil(hy, "hyphen form of resolv_conf accepted")
assert.eq(hy.nspawn.resolv_conf, "bind-host",
  "hyphen form of resolv_conf accepted (matches systemd-nspawn flag spelling)")

print("templates_provisioning.lua OK")
