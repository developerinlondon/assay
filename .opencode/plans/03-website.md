# 03 - Assay Website (assay.rs)

**Status**: PROPOSAL **Created**: 2026-02-11

---

## Context

Assay v0.3.1 is published (crates.io, Docker, GitHub Release). The domain assay.rs is purchased and
pointed to Cloudflare nameservers. We need a documentation website hosted on Cloudflare Pages.

**Requirements**:

- Modern, professional developer tool site
- Comprehensive docs (installation, API reference, examples, stdlib modules)
- Code syntax highlighting for Lua and YAML
- Dark mode
- Search
- Easy to maintain (Markdown-based content)
- Hosted on Cloudflare Pages (static)

---

## Option Comparison

```
+-----------------------------------------------------------------------------------+
|                        Static Site Generator Comparison                           |
+-----------------------------------------------------------------------------------+
| Criteria              | Starlight (Astro)   | VitePress (Vue)     | Plain HTML     |
|-----------------------|---------------------|---------------------|----------------|
| Search                | Pagefind (built-in) | MiniSearch (built-in)| Manual (Pagefind)|
| Dark mode             | Built-in            | Built-in            | Manual         |
| Syntax highlighting   | Expressive Code     | Shiki               | Manual (Prism) |
| Lua + YAML support    | Yes (100+ langs)    | Yes (200+ langs)    | Yes (Prism)    |
| Sidebar navigation    | Auto-generated      | Auto-generated      | Manual         |
| Markdown content      | MDX + components    | Vue in Markdown     | No             |
| Custom components     | React/Vue/Svelte    | Vue only            | Anything       |
| Code block features   | Titles, diff, focus | Titles, diff, focus | Basic          |
| i18n                  | 30+ languages       | Yes                 | Manual         |
| Versioning            | Manual              | Manual              | Manual         |
| Cloudflare Pages      | Perfect (static)    | Perfect (static)    | Perfect        |
| Maintenance (20+ pg)  | Low                 | Low                 | High           |
| Framework knowledge   | Minimal             | Vue helps           | HTML/CSS       |
| Build output JS       | ~50-200KB           | ~50-200KB           | ~10-20KB       |
| Lighthouse score      | 100/100             | 100/100             | 100/100        |
| Dev server            | Vite (fast)         | Vite (instant)      | None needed    |
+-----------------------+---------------------+---------------------+----------------+
```

### Real-World Usage by Similar Tools

```
+------------------------------------------------------------------------+
| Tool                 | Tech              | Type                        |
|----------------------|-------------------|-----------------------------|
| lychee (Rust CLI)    | Starlight         | Link checker                |
| Biome (Rust)         | Starlight         | Formatter/linter            |
| Tauri (Rust)         | Starlight         | Desktop framework           |
| ast-grep (Rust CLI)  | VitePress         | Code search/rewrite         |
| mise (Rust CLI)      | VitePress         | Dev environment manager     |
| Vite, Vitest, Rollup | VitePress         | JS tooling                  |
| charm.sh             | Custom HTML       | CLI tools (marketing-heavy) |
| uv (Astral)          | Custom React      | Python package manager      |
+------------------------------------------------------------------------+
```

### Recommendation: Astro Starlight

**Why Starlight over VitePress**:

1. Framework-agnostic — no Vue dependency; can use React, Svelte, or plain HTML
2. Ships zero JS by default — only loads what's needed (search, theme toggle)
3. Stronger adoption among Rust CLI tools (lychee, Biome, Tauri)
4. Expressive Code plugin has richer code block features (terminal frames, file tabs)
5. Pagefind search works offline with zero config

**Why Starlight over Plain HTML**:

1. 20+ pages of docs (API reference, 19 stdlib modules, examples) — manual nav is unsustainable
2. Built-in search is essential for API docs
3. Markdown-based content is 5x faster to write and maintain than raw HTML
4. Dark mode, mobile nav, sidebar all work out of the box

**Why NOT Starlight**:

1. No native versioning (not needed for v0.x — single version for now)
2. Requires Node.js toolchain for builds
3. Less control over design than custom HTML (mitigated by component overrides)

---

## Site Structure

```
assay.rs/
|
+-- / (Landing page - hero, value prop, install, comparison chart)
|
+-- /docs/
|   +-- /getting-started/        (install, first script, first check)
|   +-- /guides/
|   |   +-- /lua-scripts/        (writing scripts, shebang, env vars)
|   |   +-- /yaml-checks/        (check types, retry, backoff, output)
|   |   +-- /web-services/       (http.serve, routes, middleware)
|   |   +-- /database/           (connect, query, transactions)
|   |   +-- /kubernetes/         (deployment, jobs, hooks)
|   |
|   +-- /reference/
|   |   +-- /builtins/           (http, json, yaml, toml, fs, crypto, etc.)
|   |   +-- /stdlib/             (19 modules: prometheus, k8s, argocd, etc.)
|   |   +-- /cli/               (flags, env vars, exit codes)
|   |
|   +-- /examples/               (real-world recipes)
|
+-- /blog/                       (optional — release notes, tutorials)
```

---

## Content Source

Most content already exists in the README (603 lines). Migration plan:

| README Section            | Website Page                       |
| ------------------------- | ---------------------------------- |
| What is Assay / Why Assay | Landing page hero + comparison     |
| Installation              | /docs/getting-started/installation |
| Two Modes                 | /docs/getting-started/             |
| Built-in API Reference    | /docs/reference/builtins/ (split)  |
| Stdlib Modules            | /docs/reference/stdlib/ (19 pages) |
| Examples (inline)         | /docs/examples/                    |
| YAML Check Mode           | /docs/guides/yaml-checks/          |
| Architecture diagram      | /docs/reference/ or landing page   |

New content to write:

- Landing page (hero, install one-liner, comparison chart, feature cards)
- Getting started tutorial (write your first script, run it, see output)
- Per-stdlib module pages with usage examples (19 pages, mostly expand from README)
- Kubernetes deployment guide (Dockerfile, Job spec, ArgoCD hook example)

---

## Implementation Steps

| Step | What                                                | Effort |
| ---- | --------------------------------------------------- | ------ |
| 1    | Scaffold Starlight project, configure, deploy empty | 30 min |
| 2    | Landing page (hero, install, comparison, features)  | 1 hr   |
| 3    | Getting started section (install, first script)     | 30 min |
| 4    | Migrate builtin API reference from README           | 1 hr   |
| 5    | Migrate stdlib module docs (19 modules)             | 2 hr   |
| 6    | Guides (Lua scripts, YAML checks, web services)     | 1.5 hr |
| 7    | Examples page with real-world recipes               | 30 min |
| 8    | Cloudflare Pages deployment + assay.rs DNS          | 15 min |

**Total estimated agent time**: ~7 hours

---

## Cloudflare Pages Setup

```
+-------------------------------------------------------------+
|                     Deployment Flow                          |
|                                                              |
| GitHub (assay repo)                                          |
|   /website directory (or separate repo)                      |
|       |                                                      |
|       v                                                      |
| Cloudflare Pages                                             |
|   Build command: npm run build                               |
|   Output dir: dist/                                          |
|   Branch: main                                               |
|       |                                                      |
|       v                                                      |
| assay.rs (Custom domain)                                     |
|   CNAME -> <project>.pages.dev                               |
+-------------------------------------------------------------+
```

**Decision needed**: Website lives in the assay repo (`/website` directory) or a separate repo?

- **Same repo**: Simpler, docs stay with code, single PR updates both
- **Separate repo**: Cleaner separation, independent deploy, different contributors

---

## Open Questions

1. **Same repo or separate repo** for website source?
2. **Landing page design** — minimal (like lychee.cli.rs) or marketing-heavy (like charm.sh)?
3. **Blog section** — include for release notes, or skip for now?
4. **Custom domain setup** — do you have Cloudflare Pages already configured, or start fresh?
5. **Approval to proceed** with Starlight?
