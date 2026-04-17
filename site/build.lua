-- Site build script for assay.rs — assay builds its own docs.
--
-- Source (all under site/, tracked in git):
--   site/pages/*.html          — page templates with __PLACEHOLDER__ markers
--   site/partials/             — header.html, footer.html
--   site/static/*              — CSS, _headers, _redirects, llms.txt
--   docs/modules/*.md          — module documentation (single source of truth)
--   CHANGELOG.md               — release history
--
-- Output (gitignored):
--   build/site/                — ready to deploy to Cloudflare Pages
--
-- Usage: assay site/build.lua

local out = "build/site"
local modules_out = out .. "/modules"

-- =====================================================================
-- Helpers
-- =====================================================================

local function substitute(content, placeholder, replacement)
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

local function count_builtins()
  local src = fs.read("src/lua/builtins/mod.rs")
  local internal = { register_all=1, register_shell=1, register_process=1, register_disk=1, register_os=1 }
  local count = 0
  for line in src:gmatch("[^\n]+") do
    local fn_name = line:match("(register_%w+)")
    if fn_name and not internal[fn_name] and not line:match("^%s*//") and not line:match("^%s*mod ") then
      count = count + 1
    end
  end
  return count
end

-- =====================================================================
-- Phase 0: Compute variables
-- =====================================================================
local module_count = count_builtins() + #fs.glob("stdlib/**/*.lua")

local git_sha = "local-dev"
local ok, result = pcall(function()
  local tag = io.popen("git describe --tags --exact-match HEAD 2>/dev/null"):read("*l")
  if tag and #tag > 0 then return tag end
  local sha = io.popen("git rev-parse --short HEAD 2>/dev/null"):read("*l")
  if sha and #sha > 0 then return sha end
  return "local-dev"
end)
if ok then git_sha = result end

log.info("Module count: " .. module_count .. " | Git: " .. git_sha)

local header_html = fs.read("site/partials/header.html")
local footer_html = fs.read("site/partials/footer.html")

local changelog_html = ""
local cok, cmd = pcall(fs.read, "CHANGELOG.md")
if cok then
  changelog_html = markdown.to_html(cmd)
  changelog_html = changelog_html:gsub("^<h1>.-</h1>%s*", "")
  changelog_html = changelog_html:gsub("^<p>All notable.-</p>%s*", "")
end

local function apply_placeholders(html)
  html = substitute(html, "__HEADER__", header_html)
  html = substitute(html, "__FOOTER__", footer_html)
  html = substitute(html, "__GIT_SHA__", git_sha)
  html = substitute(html, "__MODULE_COUNT__", tostring(module_count))
  html = substitute(html, "__CHANGELOG_CONTENT__", changelog_html)
  return html
end

-- =====================================================================
-- Phase 1: Copy static assets
-- =====================================================================
local statics = fs.glob("site/static/*")
for _, f in ipairs(statics) do
  fs.write(out .. "/" .. f:match("([^/]+)$"), fs.read(f))
end
log.info("Copied " .. #statics .. " static assets")

-- =====================================================================
-- Phase 2: Build page templates
-- =====================================================================
local pages = fs.glob("site/pages/*.html")
for _, f in ipairs(pages) do
  fs.write(out .. "/" .. f:match("([^/]+)$"), apply_placeholders(fs.read(f)))
end
log.info("Built " .. #pages .. " pages")

-- =====================================================================
-- Phase 3: Generate per-module HTML pages
-- =====================================================================
local md_files = fs.glob("docs/modules/*.md")
fs.write(modules_out .. "/.gitkeep", "")

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
    <p><a href="/modules.html">&larr; All Modules</a></p>
{{content}}
  </main>

__FOOTER__
</body>
</html>]]

local categories = {
  { name = "Builtins (no require needed)", pattern = {"http", "serialization", "crypto", "regex", "db", "ws", "template", "async", "assert", "utilities", "fs", "markdown"} },
  { name = "Monitoring &amp; Observability", pattern = {"prometheus", "alertmanager", "loki", "grafana"} },
  { name = "Kubernetes &amp; GitOps", pattern = {"k8s", "argocd", "kargo", "flux", "traefik"} },
  { name = "Security &amp; Identity", pattern = {"vault", "openbao", "certmanager", "eso", "dex", "zitadel", "ory"} },
  { name = "Infrastructure", pattern = {"crossplane", "velero", "harbor"} },
  { name = "Data &amp; Storage", pattern = {"postgres", "s3"} },
  { name = "Feature Flags &amp; Utilities", pattern = {"unleash", "healthcheck"} },
  { name = "AI Agent &amp; Workflow", pattern = {"ai-agents", "workflow"} },
}

local modules = {}
for _, md_file in ipairs(md_files) do
  local slug = md_file:match("([^/]+)%.md$")
  local md_content = fs.read(md_file)
  local title = md_content:match("^## ([^\n]+)") or slug

  local page = module_template
  page = substitute(page, "{{title}}", title)
  page = substitute(page, "{{content}}", markdown.to_html(md_content))
  page = apply_placeholders(page)

  fs.write(modules_out .. "/" .. slug .. ".html", page)
  modules[#modules + 1] = { slug = slug, title = title }
end
log.info("Generated " .. #modules .. " module pages")

-- =====================================================================
-- Phase 4: Generate modules.html index
-- =====================================================================
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

local sections = {}
for _, cat in ipairs(categories) do
  local items = modules_in_category(cat.pattern)
  if #items > 0 then
    sections[#sections + 1] = '    <h2>' .. cat.name .. '</h2>\n    <ul>\n' .. items .. '\n    </ul>'
  end
end

local index_html = [[<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Assay — Module Reference</title>
  <meta name="description" content="Complete reference for all ]] .. tostring(module_count) .. [[ Assay modules.">
  <link rel="stylesheet" href="style.css">
</head>
<body class="page-modules">
__HEADER__

  <main>
    <h1>Module Reference</h1>
    <p style="font-size: 1.15rem; color: var(--text-secondary); margin-bottom: 1.5rem;">
      ]] .. tostring(module_count) .. [[ modules, zero dependencies. Use <code>assay context &lt;query&gt;</code> for LLM-ready docs.
    </p>

    <pre><code>assay modules                    # list all modules
assay context "grafana health"   # get detailed docs for LLM</code></pre>

]] .. table.concat(sections, "\n\n") .. [[

  </main>

__FOOTER__
</body>
</html>]]

fs.write(out .. "/modules.html", apply_placeholders(index_html))

-- =====================================================================
-- Phase 4b: Generate per-example HTML pages + examples.html index
-- =====================================================================
-- Each subdirectory of examples/workflows/ ships a README.md and one or
-- more *.lua files. We render the README + a syntax-highlighted view of
-- each lua file so a visitor can see exactly what the example does
-- without having to clone the repo.

local examples_out = out .. "/examples"
local example_dirs = fs.glob("examples/workflows/*/")
fs.write(examples_out .. "/.gitkeep", "")

local example_template = [[<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Assay — Example: {{title}}</title>
  <meta name="description" content="Runnable assay workflow example: {{title}}">
  <link rel="stylesheet" href="/style.css">
</head>
<body class="page-modules">
__HEADER__

  <main>
    <p><a href="/examples.html">&larr; All Examples</a></p>
{{content}}
{{lua_files}}
  </main>

__FOOTER__
</body>
</html>]]

local examples_index_entries = {}

for _, dir in ipairs(example_dirs) do
  local slug = dir:match("examples/workflows/([^/]+)/?$")
  if slug then
    local readme_path = "examples/workflows/" .. slug .. "/README.md"
    local readme_ok, readme_md = pcall(fs.read, readme_path)
    if readme_ok then
      local title = readme_md:match("^# ([^\n]+)") or slug
      local content_html = markdown.to_html(readme_md)
      -- Strip the leading <h1> since we render it differently
      content_html = content_html:gsub("^<h1>.-</h1>%s*", "<h1>" .. title .. "</h1>")

      -- Inline each .lua file in the example directory so visitors can
      -- read the worker source without leaving the page.
      local lua_files_html = ""
      local lua_files = fs.glob("examples/workflows/" .. slug .. "/*.lua")
      if #lua_files > 0 then
        lua_files_html = "<h2>Source</h2>"
        for _, lua_path in ipairs(lua_files) do
          local lua_name = lua_path:match("([^/]+)$")
          local lua_src = fs.read(lua_path)
          -- Basic HTML-escape for the code block
          lua_src = lua_src:gsub("&", "&amp;"):gsub("<", "&lt;"):gsub(">", "&gt;")
          lua_files_html = lua_files_html
            .. '<h3><code>' .. lua_name .. '</code></h3>'
            .. '<pre><code>' .. lua_src .. '</code></pre>'
        end
      end

      local page = example_template
      page = substitute(page, "{{title}}", title)
      page = substitute(page, "{{content}}", content_html)
      page = substitute(page, "{{lua_files}}", lua_files_html)
      page = apply_placeholders(page)

      fs.write(examples_out .. "/" .. slug .. ".html", page)

      -- Pull the first paragraph of the README for the index summary
      local summary = readme_md:match("# [^\n]+\n+## What it does\n+([^#]+)")
        or readme_md:match("# [^\n]+\n+([^\n#]+)")
        or ""
      summary = summary:gsub("^%s+", ""):gsub("%s+$", "")
      examples_index_entries[#examples_index_entries + 1] = {
        slug = slug, title = title, summary = summary,
      }
    end
  end
end
log.info("Generated " .. #examples_index_entries .. " example pages")

-- examples.html index page
local index_items = {}
for _, e in ipairs(examples_index_entries) do
  index_items[#index_items + 1] = string.format([[
      <a href="/examples/%s.html" class="card" style="display: block; text-decoration: none; color: var(--text); margin-bottom: 1rem;">
        <h3 style="margin-top: 0; color: var(--accent);">%s</h3>
        <p class="text-muted" style="margin-bottom: 0;">%s</p>
      </a>]], e.slug, e.title, e.summary)
end

local examples_index_html = [[<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Assay — Workflow Examples</title>
  <meta name="description" content="Runnable assay workflow engine examples: hello, signals, cron, child workflows.">
  <link rel="stylesheet" href="style.css">
</head>
<body class="page-modules">
__HEADER__

  <main>
    <h1>Workflow Examples</h1>
    <p style="font-size: 1.05rem; color: var(--text-secondary); margin-bottom: 1.5rem;">
      Runnable examples of the <code>assay serve</code> workflow engine.
      Each links to its README and source. Clone the repo and run
      <code>assay run examples/workflows/&lt;name&gt;/worker.lua</code>
      with <code>assay serve</code> on :8080 to try them.
    </p>
]] .. table.concat(index_items, "\n") .. [[

  </main>

__FOOTER__
</body>
</html>]]
fs.write(out .. "/examples.html", apply_placeholders(examples_index_html))

-- =====================================================================
-- Phase 5: Generate llms-full.txt
-- =====================================================================
local llms = { [[# Assay

> Assay is a ~9 MB static binary that runs Lua scripts in Kubernetes. It replaces 50-250 MB
> Python/Node/kubectl containers. One binary handles HTTP, database, crypto, WebSocket, and
> Kubernetes-native and AI agent service integrations. No `require()` for builtins — they are global.
> Stdlib modules use `require("assay.<name>")` then `M.client(url, opts)` → `c:method()`.
> Run `assay context <query>` to get LLM-ready method signatures for any module.
>
> Client pattern: `local mod = require("assay.<name>")` → `local c = mod.client(url, opts)` → `c:method()`.
> Auth varies: `{token="..."}`, `{api_key="..."}`, `{username="...", password="..."}`.

## Getting Started

- [README](https://github.com/developerinlondon/assay/blob/main/README.md): Installation, quick start, examples
- [SKILL.md](https://github.com/developerinlondon/assay/blob/main/SKILL.md): LLM agent integration guide
- [GitHub](https://github.com/developerinlondon/assay): Source code and issues

]] }

for _, md_file in ipairs(md_files) do
  llms[#llms + 1] = fs.read(md_file)
  llms[#llms + 1] = "\n\n"
end

-- Workflow engine examples — show real runnable code so agents can
-- pattern-match what a worker script + handler looks like.
if #example_dirs > 0 then
  llms[#llms + 1] = "## Workflow examples\n\n"
  for _, e in ipairs(examples_index_entries) do
    llms[#llms + 1] = "### " .. e.title .. "\n\n"
    local readme_path = "examples/workflows/" .. e.slug .. "/README.md"
    local rok, rmd = pcall(fs.read, readme_path)
    if rok then
      llms[#llms + 1] = rmd .. "\n\n"
    end
    local lua_files = fs.glob("examples/workflows/" .. e.slug .. "/*.lua")
    for _, lua_path in ipairs(lua_files) do
      local lua_name = lua_path:match("([^/]+)$")
      llms[#llms + 1] = "**`" .. lua_name .. "`**\n\n```lua\n"
      llms[#llms + 1] = fs.read(lua_path)
      llms[#llms + 1] = "\n```\n\n"
    end
  end
end

llms[#llms + 1] = [[## Optional
- [Crates.io](https://crates.io/crates/assay-lua): Use Assay as a Rust crate
- [Docker](https://github.com/developerinlondon/assay/pkgs/container/assay): ghcr.io/developerinlondon/assay:latest
- [Agent Guides](https://assay.rs/agent-guides.html): Claude Code, Cursor, Windsurf, Cline, OpenCode
- [Changelog](https://github.com/developerinlondon/assay/releases): Release history
]]

fs.write(out .. "/llms-full.txt", table.concat(llms))

log.info("Done. Output at " .. out .. "/")
