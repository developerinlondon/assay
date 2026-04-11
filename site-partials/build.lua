-- Site build script for assay.rs — assay builds its own docs.
--
-- Source of truth: docs/modules/*.md
-- Generates:
--   site/modules/<name>.html   — per-module HTML pages
--   site/modules.html          — auto-generated module index
--   site/llms-full.txt         — all modules concatenated for LLM agents
--
-- Also substitutes placeholders in all site/*.html:
--   __HEADER__        → site-partials/header.html
--   __FOOTER__        → site-partials/footer.html
--   __GIT_SHA__       → git tag or short SHA
--   __MODULE_COUNT__  → computed from builtins + stdlib
--
-- Usage: assay site-partials/build.lua

local site_dir = "site"
local partials_dir = "site-partials"
local docs_dir = "docs/modules"
local modules_out = site_dir .. "/modules"

-- =====================================================================
-- Helpers
-- =====================================================================

--- Replace all occurrences of a placeholder with a string value.
local function substitute(content, placeholder, replacement)
  -- Use plain string find/replace (no patterns)
  local result = {}
  local pos = 1
  while true do
    local i, j = content:find(placeholder, pos, true)
    if not i then
      result[#result + 1] = content:sub(pos)
      break
    end
    result[#result + 1] = content:sub(pos, i - 1)
    result[#result + 1] = replacement
    pos = j + 1
  end
  return table.concat(result)
end

--- Count user-facing builtins by reading mod.rs register calls.
--- Excludes internal modules (shell, process, disk, os) and counts
--- temporal + temporal_worker as one module.
local function count_builtins()
  local src = fs.read("src/lua/builtins/mod.rs")
  local internal = { register_all=1, register_shell=1, register_process=1, register_disk=1, register_os=1, register_temporal_worker=1 }
  local count = 0
  for line in src:gmatch("[^\n]+") do
    local fn_name = line:match("(register_%w+)")
    if fn_name and not internal[fn_name] and not line:match("^%s*//") and not line:match("^%s*mod ") then
      count = count + 1
    end
  end
  return count
end

--- Count stdlib modules by globbing .lua files.
local function count_stdlib()
  local files = fs.glob("stdlib/**/*.lua")
  return #files
end

-- =====================================================================
-- Phase 0: Compute variables
-- =====================================================================
local builtin_count = count_builtins()
local stdlib_count = count_stdlib()
local module_count = builtin_count + stdlib_count

-- Get git version
local git_sha = "local-dev"
local ok, result = pcall(function()
  local tag = io.popen("git describe --tags --exact-match HEAD 2>/dev/null"):read("*l")
  if tag and #tag > 0 then return tag end
  local sha = io.popen("git rev-parse --short HEAD 2>/dev/null"):read("*l")
  if sha and #sha > 0 then return sha end
  return "local-dev"
end)
if ok then git_sha = result end

log.info("Module count: " .. module_count .. " (" .. builtin_count .. " builtins + " .. stdlib_count .. " stdlib)")
log.info("Git version: " .. git_sha)

-- Load partials
local header_html = fs.read(partials_dir .. "/header.html")
local footer_html = fs.read(partials_dir .. "/footer.html")

-- Build changelog HTML from CHANGELOG.md
local changelog_html = ""
local changelog_ok, changelog_md = pcall(fs.read, "CHANGELOG.md")
if changelog_ok then
  changelog_html = markdown.to_html(changelog_md)
  -- Strip the top-level heading and intro paragraph
  changelog_html = changelog_html:gsub("^<h1>.-</h1>%s*", "")
  changelog_html = changelog_html:gsub("^<p>All notable.-</p>%s*", "")
end

--- Apply all placeholder substitutions to an HTML string.
local function apply_placeholders(html)
  html = substitute(html, "__HEADER__", header_html)
  html = substitute(html, "__FOOTER__", footer_html)
  html = substitute(html, "__GIT_SHA__", git_sha)
  html = substitute(html, "__MODULE_COUNT__", tostring(module_count))
  html = substitute(html, "__CHANGELOG_CONTENT__", changelog_html)
  return html
end

-- =====================================================================
-- Phase 1: Generate per-module HTML pages from docs/modules/*.md
-- =====================================================================
local md_files = fs.glob(docs_dir .. "/*.md")

if #md_files > 0 then
  -- Ensure output directory exists
  pcall(fs.read, modules_out .. "/.keep") -- will fail but that's ok
  -- Create modules directory
  fs.write(modules_out .. "/.keep", "")
  fs.remove(modules_out .. "/.keep")

  local module_template = [[<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Assay — {{title}}</title>
  <meta name="description" content="Assay module reference: {{title}}">
  <link rel="stylesheet" href="../style.css">
</head>
<body class="page-modules">
__HEADER__

  <main>
    <p><a href="../modules.html">&larr; All Modules</a></p>
{{content}}
  </main>

__FOOTER__
</body>
</html>]]

  -- Category definitions for the index page
  local categories = {
    { name = "Builtins (no require needed)", pattern = {"http", "serialization", "crypto", "regex", "db", "ws", "template", "async", "assert", "utilities", "fs", "markdown"} },
    { name = "Monitoring &amp; Observability", pattern = {"prometheus", "alertmanager", "loki", "grafana"} },
    { name = "Kubernetes &amp; GitOps", pattern = {"k8s", "argocd", "kargo", "flux", "traefik"} },
    { name = "Security &amp; Identity", pattern = {"vault", "openbao", "certmanager", "eso", "dex", "zitadel", "ory"} },
    { name = "Infrastructure", pattern = {"crossplane", "velero", "temporal", "harbor"} },
    { name = "Data &amp; Storage", pattern = {"postgres", "s3"} },
    { name = "Feature Flags &amp; Utilities", pattern = {"unleash", "healthcheck"} },
    { name = "AI Agent &amp; Workflow", pattern = {"ai-agents"} },
  }

  -- Build module entries: { slug, title }
  local modules = {}
  for _, md_file in ipairs(md_files) do
    local slug = md_file:match("([^/]+)%.md$")
    local md_content = fs.read(md_file)
    local title = md_content:match("^## ([^\n]+)") or slug

    -- Convert markdown to HTML
    local html_body = markdown.to_html(md_content)

    -- Build the page
    local page = module_template
    page = substitute(page, "{{title}}", title)
    page = substitute(page, "{{content}}", html_body)
    page = apply_placeholders(page)

    fs.write(modules_out .. "/" .. slug .. ".html", page)
    modules[#modules + 1] = { slug = slug, title = title }
  end

  log.info("Generated " .. #modules .. " module pages")

  -- ---------------------------------------------------------------
  -- Generate modules.html index page
  -- ---------------------------------------------------------------
  local function modules_in_category(cat_pattern)
    local items = {}
    for _, m in ipairs(modules) do
      for _, p in ipairs(cat_pattern) do
        if m.slug == p then
          items[#items + 1] = '        <li><a href="modules/' .. m.slug .. '.html">' .. m.title .. '</a></li>'
          break
        end
      end
    end
    return table.concat(items, "\n")
  end

  local index_sections = {}
  for _, cat in ipairs(categories) do
    local items = modules_in_category(cat.pattern)
    if #items > 0 then
      index_sections[#index_sections + 1] = '    <h2>' .. cat.name .. '</h2>\n    <ul>\n' .. items .. '\n    </ul>'
    end
  end

  local index_html = [[<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Assay — Module Reference</title>
  <meta name="description" content="Complete reference for all __MODULE_COUNT__ Assay modules — Rust builtins and embedded Lua stdlib modules, zero dependencies.">
  <link rel="stylesheet" href="style.css">
</head>
<body class="page-modules">
__HEADER__

  <main>
    <h1>Module Reference</h1>
    <p style="font-size: 1.15rem; color: var(--text-secondary); margin-bottom: 1.5rem;">
      __MODULE_COUNT__ modules, zero dependencies. Use <code>assay context &lt;query&gt;</code> for LLM-ready docs on any module.
    </p>

    <pre><code>assay modules                    # list all modules
assay context "grafana health"   # get detailed docs for LLM</code></pre>

]] .. table.concat(index_sections, "\n\n") .. [[

  </main>

__FOOTER__
</body>
</html>]]

  index_html = apply_placeholders(index_html)
  fs.write(site_dir .. "/modules.html", index_html)

  -- ---------------------------------------------------------------
  -- Generate llms-full.txt from all markdown files
  -- ---------------------------------------------------------------
  local llms_parts = {}
  llms_parts[#llms_parts + 1] = [[# Assay

> Assay is a ~9 MB static binary that runs Lua scripts in Kubernetes. It replaces 50-250 MB
> Python/Node/kubectl containers. One binary handles HTTP, database, crypto, WebSocket, and
> Kubernetes-native and AI agent service integrations. No `require()` for builtins — they are global.
> Stdlib modules use `require("assay.<name>")` then `M.client(url, opts)` → `c:method()`.
> Run `assay context <query>` to get LLM-ready method signatures for any module.
> HTTP responses are `{status, body, headers}` tables. Errors raised via `error()` — use `pcall()`.
>
> Client pattern: `local mod = require("assay.<name>")` → `local c = mod.client(url, opts)` → `c:method()`.
> Auth varies: `{token="..."}`, `{api_key="..."}`, `{username="...", password="..."}`.
> Error format: `"<module>: <METHOD> <path> HTTP <status>: <body>"`.
> 404 returns nil for most client methods.

## Getting Started

- [README](https://github.com/developerinlondon/assay/blob/main/README.md): Installation, quick start, examples
- [SKILL.md](https://github.com/developerinlondon/assay/blob/main/SKILL.md): LLM agent integration guide
- [GitHub](https://github.com/developerinlondon/assay): Source code and issues

]]

  for _, md_file in ipairs(md_files) do
    llms_parts[#llms_parts + 1] = fs.read(md_file)
    llms_parts[#llms_parts + 1] = "\n\n"
  end

  llms_parts[#llms_parts + 1] = [[## Optional
- [Crates.io](https://crates.io/crates/assay-lua): Use Assay as a Rust crate in your own projects
- [Docker](https://github.com/developerinlondon/assay/pkgs/container/assay): ghcr.io/developerinlondon/assay:latest (~9MB compressed)
- [Agent Guides](https://assay.rs/agent-guides.html): Integration guides for Claude Code, Cursor, Windsurf, Cline, OpenCode
- [Changelog](https://github.com/developerinlondon/assay/releases): Release history
]]

  fs.write(site_dir .. "/llms-full.txt", table.concat(llms_parts))
end

-- =====================================================================
-- Phase 2: Substitute placeholders in existing site pages
-- =====================================================================
log.info("Substituting placeholders in site/*.html")
local html_files = fs.glob(site_dir .. "/*.html")
for _, f in ipairs(html_files) do
  local content = fs.read(f)
  if content:find("__HEADER__", 1, true)
    or content:find("__FOOTER__", 1, true)
    or content:find("__GIT_SHA__", 1, true)
    or content:find("__MODULE_COUNT__", 1, true) then
    fs.write(f, apply_placeholders(content))
  end
end

log.info("Done. Site rendered at " .. site_dir .. "/")
