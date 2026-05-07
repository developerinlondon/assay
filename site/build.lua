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
  -- Path changed in v0.13.0 when the root binary moved to crates/assay/.
  local src = fs.read("crates/assay/src/lua/builtins/mod.rs")
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
-- =====================================================================
local module_count = count_builtins() + #fs.glob("crates/assay/stdlib/**/*.lua")

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
-- =====================================================================
local statics = fs.glob("site/static/*")
for _, f in ipairs(statics) do
  fs.write(out .. "/" .. f:match("([^/]+)$"), fs.read(f))
end
log.info("Copied " .. #statics .. " static assets")

-- =====================================================================
-- =====================================================================
local pages = fs.glob("site/pages/*.html")
for _, f in ipairs(pages) do
  fs.write(out .. "/" .. f:match("([^/]+)$"), apply_placeholders(fs.read(f)))
end
log.info("Built " .. #pages .. " pages")

-- =====================================================================
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

-- Display order for categories. Any category found in frontmatter that is
-- not in this list is appended at the end in alphabetical order. Adding a
-- new category to a module's frontmatter is the only step needed to make
-- it appear; ordering it explicitly is optional.
local category_order = {
  "Builtins",
  "Monitoring & Observability",
  "Kubernetes & GitOps",
  "Security & Identity",
  "Infrastructure",
  "Data & Storage",
  "Feature Flags & Health",
  "Text, URLs & Versions",
  "AI Agents & Workflow",
}

-- Display name override (e.g. for the modules.html headers we want
-- "Builtins (no require needed)" instead of just "Builtins").
local category_display = {
  ["Builtins"] = "Builtins (no require needed)",
}

-- Parse YAML-ish frontmatter from a markdown source. Returns
-- (fields_table, body_without_frontmatter). Fields are key/value strings;
-- only top-level scalar values are supported (sufficient for category +
-- tagline). If the file has no frontmatter, fields is empty and body is
-- the original content.
local function parse_frontmatter(src)
  local fields = {}
  local rest = src:match("^%-%-%-\n(.-\n)%-%-%-\n+(.*)$")
  if not rest then
    return fields, src
  end
  local header = src:match("^%-%-%-\n(.-)\n%-%-%-\n")
  local body   = src:match("^%-%-%-\n.-\n%-%-%-\n+(.*)$") or src
  for line in (header or ""):gmatch("[^\n]+") do
    local k, v = line:match("^([%w_]+)%s*:%s*(.*)$")
    if k then fields[k] = v end
  end
  return fields, body
end

-- Take the first paragraph after the H2 header as a one-line tagline.
-- Newlines collapsed to single spaces. Used for README/SKILL table cells.
local function extract_tagline(body)
  local first_para = body:match("^## [^\n]+\n+([^\n]+(\n[^\n]+)*)") or ""
  -- The match is greedy across blank lines, so trim at first double-newline.
  first_para = first_para:match("^([^\n].-)\n\n") or first_para
  return (first_para:gsub("\n", " "):gsub("%s+", " "):gsub("^%s+", ""):gsub("%s+$", ""))
end

local modules = {}
for _, md_file in ipairs(md_files) do
  local slug = md_file:match("([^/]+)%.md$")
  local raw  = fs.read(md_file)
  local fields, body = parse_frontmatter(raw)
  local title    = body:match("^## ([^\n]+)") or slug
  local category = fields.category or "Uncategorised"
  local tagline  = fields.tagline or extract_tagline(body)

  local page = module_template
  page = substitute(page, "{{title}}", title)
  page = substitute(page, "{{content}}", markdown.to_html(body))
  page = apply_placeholders(page)

  fs.write(modules_out .. "/" .. slug .. ".html", page)
  modules[#modules + 1] = {
    slug = slug, title = title, category = category, tagline = tagline,
  }
end
log.info("Generated " .. #modules .. " module pages")

-- =====================================================================
-- =====================================================================
-- Build categories dynamically from frontmatter, ordered by
-- `category_order` (then any unknown categories alphabetically).
local by_category = {}
for _, m in ipairs(modules) do
  by_category[m.category] = by_category[m.category] or {}
  table.insert(by_category[m.category], m)
end
for cat, list in pairs(by_category) do
  table.sort(list, function(a, b) return a.slug < b.slug end)
end

local function ordered_categories()
  local seen = {}
  local result = {}
  for _, cat in ipairs(category_order) do
    if by_category[cat] then
      result[#result + 1] = cat
      seen[cat] = true
    end
  end
  local extras = {}
  for cat, _ in pairs(by_category) do
    if not seen[cat] then extras[#extras + 1] = cat end
  end
  table.sort(extras)
  for _, cat in ipairs(extras) do result[#result + 1] = cat end
  return result
end

local html_escape = function(s)
  return (s:gsub("&", "&amp;"):gsub("<", "&lt;"):gsub(">", "&gt;"))
end

local sections = {}
for _, cat in ipairs(ordered_categories()) do
  local list = by_category[cat]
  local items = {}
  for _, m in ipairs(list) do
    items[#items + 1] = '        <li><a href="modules/' .. m.slug .. '.html">'
                     .. html_escape(m.title) .. '</a></li>'
  end
  local display = category_display[cat] or cat
  sections[#sections + 1] = '    <h2>' .. html_escape(display) .. '</h2>\n'
                          .. '    <ul>\n' .. table.concat(items, "\n") .. '\n    </ul>'
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

-- =====================================================================
-- README.md / optional SKILL.md auto-rewrite
-- =====================================================================
-- Modules + categories live in `docs/modules/<slug>.md` frontmatter
-- (`category:` field). The stdlib table in README is generated from
-- that, replacing whatever sits between the BEGIN/END markers. SKILL.md
-- can opt into the same rewrite by adding those markers. Run
-- `assay site/build.lua` after adding/changing a module; CI should fail
-- if `git diff README.md SKILL.md` is non-empty afterwards.

local function render_stdlib_md_table()
  local lines = {
    "<!-- BEGIN STDLIB TABLE -->",
    "<!-- Generated by site/build.lua from docs/modules/*.md frontmatter — do not edit by hand. -->",
    "",
    "| Module | Description |",
    "| --- | --- |",
  }
  for _, cat in ipairs(ordered_categories()) do
    if cat ~= "Builtins" then
      lines[#lines + 1] = "| **" .. cat .. "** | |"
      for _, m in ipairs(by_category[cat]) do
        local desc = m.tagline or ""
        -- Pipes inside table cells must be escaped.
        desc = desc:gsub("|", "\\|")
        lines[#lines + 1] = "| `assay." .. m.slug .. "` | " .. desc .. " |"
      end
    end
  end
  lines[#lines + 1] = ""
  -- Note: the end-marker is NOT appended here. replace_block's post_pat
  -- captures the end-marker into `post`, so prepending it again here
  -- caused the marker to be doubled on every regen run.
  return table.concat(lines, "\n")
end

local function replace_block(path, begin_marker, end_marker, replacement, opts)
  opts = opts or {}
  local ok, src = pcall(fs.read, path)
  if not ok then
    if opts.optional then
      log.info(path .. ": optional file not found, skipping")
    else
      log.warn(path .. ": not found, skipping")
    end
    return false
  end
  local pre_pat  = "^(.-)(" .. begin_marker:gsub("%-", "%%-") .. ")"
  local post_pat = "(" .. end_marker:gsub("%-", "%%-") .. ")(.*)$"
  local pre  = src:match(pre_pat)
  local post = src:match(post_pat)
  if not pre or not post then
    local msg = path .. ": markers not found, skipping (add `"
                .. begin_marker .. "` ... `" .. end_marker .. "`)"
    if opts.optional then
      log.info(msg)
    else
      log.warn(msg)
    end
    return false
  end
  local new = pre .. replacement .. post
  if new == src then
    log.info(path .. ": stdlib table unchanged")
    return false
  end
  fs.write(path, new)
  log.info(path .. ": stdlib table rewritten")
  return true
end

local table_md = render_stdlib_md_table()
replace_block("README.md", "<!-- BEGIN STDLIB TABLE -->", "<!-- END STDLIB TABLE -->", table_md)
-- SKILL.md does not currently host a stdlib table; if you want one,
-- add the BEGIN/END markers anywhere in the file and rerun this script.
replace_block("SKILL.md",  "<!-- BEGIN STDLIB TABLE -->", "<!-- END STDLIB TABLE -->", table_md, { optional = true })

log.info("Done. Output at " .. out .. "/")
