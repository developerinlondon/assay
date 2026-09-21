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
  -- Note: the end-marker is NOT appended here. replace_block splices the
  -- file back on from the end-marker onwards, so emitting it again here
  -- doubled the marker on every regen run.
  return table.concat(lines, "\n")
end

-- =====================================================================
-- AGENTS.md stdlib table
-- =====================================================================
-- AGENTS.md is read by an agent that is about to write a script, so its
-- table has to name things `require()` actually resolves. The README
-- table above cannot: it is keyed on `docs/modules/<slug>.md` filenames,
-- which are doc-page names rather than module names, so it emits rows
-- like `assay.aws-ec2` and `assay.oci` while missing every submodule and
-- roughly twenty top-level modules that have no doc page.
--
-- The modules themselves are the complete source, so AGENTS.md is
-- generated from their `--- @module` / `--- @description` / `--- @category`
-- headers — the same tags `assay context` reads at runtime.

-- Split a source file into lines, keeping empty ones. `gmatch("[^\n]+")`
-- silently skips blank lines, which would let header parsing run past the
-- end of a doc block into unrelated `---` comments further down.
local function source_lines(src)
  local lines, pos = {}, 1
  while true do
    local nl = src:find("\n", pos, true)
    if not nl then
      lines[#lines + 1] = src:sub(pos)
      return lines
    end
    lines[#lines + 1] = src:sub(pos, nl - 1)
    pos = nl + 1
  end
end

-- Parse the `--- @tag value` header at the top of a Lua module. Mirrors
-- `parse_header_tags` in crates/assay/src/metadata.rs: read `---` lines
-- from the top, stop at the first line that is not one, last occurrence
-- of a tag wins. No stdlib module declares a tag twice today, so the
-- last-wins rule only matters for staying honest about the mirror.
local function parse_lua_header(src)
  local tags = {}
  for _, line in ipairs(source_lines(src)) do
    local trimmed = line:match("^%s*(.-)%s*$")
    if trimmed:sub(1, 3) ~= "---" then break end
    local after = trimmed:gsub("^%-+", ""):match("^%s*(.-)%s*$")
    local tag, value = after:match("^@(%S+)%s+(.*)$")
    if tag then
      tags[tag] = value:match("^%s*(.-)%s*$")
    end
  end
  return tags
end

-- `@description` is a full paragraph — apify's runs to ~290 characters —
-- and a table cell wants a line. Keep whole sentences while they fit the
-- budget: cutting unconditionally at the first one would reduce half the
-- table to "Grafana monitoring and dashboards." and throw away the list of
-- what the module actually covers, which is the part an agent is reading
-- the table for.
local DESC_MAX = 160

-- Abbreviations whose trailing period is followed by a space without
-- ending a sentence. Without this, `assay.ory` renders as "…submodules
-- directly (e.g." — a cell cut off mid-parenthesis.
local NOT_SENTENCE_END = {
  ["e.g."] = true, ["i.e."] = true, ["etc."] = true, ["cf."] = true, ["vs."] = true,
}

local function ends_sentence(upto)
  local word = upto:match("([%w%.]+)$")
  return not (word and NOT_SENTENCE_END[word:lower()])
end

-- A word-boundary cut lands wherever the budget runs out, which can be in
-- the middle of a parenthetical or a code span: `assay.engine.auth` ended
-- "…and admin (users, sessions, OIDC…". Walk back to just before the first
-- opener that never closed, so the cell reads "…and admin…" instead.
local PAIRS = { ["("] = ")", ["["] = "]" }

local function drop_unclosed(s)
  local stack = {}
  local backtick_open = nil
  for i = 1, #s do
    local ch = s:sub(i, i)
    if ch == "`" then
      -- Backticks are their own closer, so one span toggles rather than nests.
      backtick_open = backtick_open and nil or i
    elseif not backtick_open then
      if PAIRS[ch] then
        stack[#stack + 1] = { ch = ch, at = i }
      elseif #stack > 0 and ch == PAIRS[stack[#stack].ch] then
        stack[#stack] = nil
      end
    end
  end
  local first_unclosed = backtick_open
  if stack[1] and (not first_unclosed or stack[1].at < first_unclosed) then
    first_unclosed = stack[1].at
  end
  if not first_unclosed then return s end
  -- Never return nothing: an all-bracket prefix keeps the original cut.
  local trimmed = s:sub(1, first_unclosed - 1):match("^(.-)%s*$")
  return #trimmed > 0 and trimmed or s
end

local function trim_tail(s)
  return (s:gsub("[%s,;:]+$", ""))
end

local function one_line_description(desc)
  desc = (desc or ""):gsub("%s+", " "):match("^%s*(.-)%s*$")
  if #desc <= DESC_MAX then return desc end

  -- `()%.%s` yields the offset of each period followed by a space. Periods
  -- inside `assay.k8s`, `v0.3.0` or `/api/v1/…` are not followed by a
  -- space, so they never split.
  local kept
  for stop in desc:gmatch("()%.%s") do
    if ends_sentence(desc:sub(1, stop)) then
      if kept and stop > DESC_MAX then break end
      kept = desc:sub(1, stop)
      if stop > DESC_MAX then break end
    end
  end
  if kept and #kept <= DESC_MAX then return kept end

  -- A single sentence that overruns on its own. Cut on a word boundary,
  -- which also keeps the cut off the middle of a multi-byte character.
  local first = kept or desc
  local cut = first:sub(1, DESC_MAX):match("^(.*)%s%S*$") or first:sub(1, DESC_MAX)
  return trim_tail(drop_unclosed(cut)) .. "…"
end

-- `@category` slugs as the modules declare them, in display order. A slug
-- that is not listed here lands under "Other" rather than failing the
-- build; give it a row below to promote it to a section of its own.
local stdlib_category_order = {
  "core", "observability", "kubernetes", "gitops", "identity", "secrets",
  "cloud", "host", "data", "devtools", "registries", "saas", "comms",
}

local stdlib_category_display = {
  core          = "Core (text, URLs, versions)",
  observability = "Monitoring & Observability",
  kubernetes    = "Kubernetes",
  gitops        = "GitOps",
  identity      = "Security & Identity",
  secrets       = "Secrets & Vaults",
  cloud         = "Cloud & Network",
  host          = "Host & Linux",
  data          = "Data & Storage",
  devtools      = "Developer Tools",
  registries    = "Registries & Company Data",
  saas          = "SaaS & Business",
  comms         = "Email & Comms",
}

local OTHER = "__other__"

local function render_agents_stdlib_table()
  local mods = {}
  for _, path in ipairs(fs.glob("crates/assay/stdlib/**/*.lua")) do
    local tags = parse_lua_header(fs.read(path))
    -- No `@module` means the file is not requirable on its own
    -- (stdlib/engine/workflow/ctx.lua is an internal helper table).
    if tags.module and #tags.module > 0 then
      mods[#mods + 1] = {
        name        = tags.module,
        description = one_line_description(tags.description),
        category    = tags.category or "",
      }
    end
  end
  table.sort(mods, function(a, b) return a.name < b.name end)

  local by_slug = {}
  for _, m in ipairs(mods) do
    local slug = stdlib_category_display[m.category] and m.category or OTHER
    by_slug[slug] = by_slug[slug] or {}
    table.insert(by_slug[slug], m)
  end

  local lines = {
    "<!-- BEGIN STDLIB TABLE -->",
    "<!-- Generated by site/build.lua from the @module/@description/@category doc blocks in crates/assay/stdlib/**/*.lua — do not edit by hand. -->",
    "",
    -- The count lives inside the block so it cannot drift from the rows.
    tostring(#mods) .. ' embedded Lua modules loaded via `require("assay.<name>")`:',
    "",
    "| Module | Description |",
    "| --- | --- |",
  }

  local function emit(slug, display)
    if not by_slug[slug] then return end
    lines[#lines + 1] = "| **" .. display .. "** | |"
    for _, m in ipairs(by_slug[slug]) do
      -- Pipes inside table cells must be escaped.
      lines[#lines + 1] = "| `" .. m.name .. "` | " .. m.description:gsub("|", "\\|") .. " |"
    end
  end

  for _, slug in ipairs(stdlib_category_order) do
    emit(slug, stdlib_category_display[slug])
  end
  emit(OTHER, "Other")

  lines[#lines + 1] = ""
  -- As above: the end-marker is spliced back by replace_block, not emitted here.
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
  -- Find a marker only where it stands alone on its own line. AGENTS.md
  -- names both markers mid-sentence in the section that documents this
  -- mechanism; matching that prose rewrites the wrong part of the file.
  local function find_own_line(marker, from)
    local pos = from
    while true do
      local s, e = src:find(marker, pos, true)
      if not s then return nil end
      local before = s == 1 or src:sub(s - 1, s - 1) == "\n"
      local after  = e == #src or src:sub(e + 1, e + 1) == "\n"
      if before and after then return s, e end
      pos = e + 1
    end
  end

  local begin_start, begin_end = find_own_line(begin_marker, 1)
  local end_start = begin_end and find_own_line(end_marker, begin_end + 1)
  if not begin_start or not end_start then
    local msg = path .. ": markers not found, skipping (add `"
                .. begin_marker .. "` ... `" .. end_marker .. "`)"
    if opts.optional then
      log.info(msg)
    else
      log.warn(msg)
    end
    return false
  end
  -- `replacement` re-emits the begin-marker and stops short of the
  -- end-marker, so the tail is spliced back on from the end-marker itself.
  -- Everything below the block is preserved.
  local new = src:sub(1, begin_start - 1) .. replacement .. src:sub(end_start)
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

-- AGENTS.md carries its own table from a different source (the module doc
-- blocks, not docs/modules frontmatter). The count printed inside that
-- block is the number of `@module` declarations under crates/assay/stdlib/;
-- it is deliberately lower than `module_count` above, which adds the
-- builtins — global, no `require()` — and counts every .lua file rather
-- than only the requirable ones. The two are not meant to agree.
replace_block("AGENTS.md", "<!-- BEGIN STDLIB TABLE -->", "<!-- END STDLIB TABLE -->",
              render_agents_stdlib_table())

log.info("Done. Output at " .. out .. "/")
