#!/usr/bin/env bash
# Build script for the assay.rs static site.
#
# Source of truth: docs/modules/*.md
# Generates:
#   site/modules/<name>.html   — per-module HTML pages
#   site/modules.html          — module index page (auto-generated)
#   site/llms-full.txt         — all modules concatenated for LLM agents
#
# Also substitutes partials (__HEADER__, __FOOTER__, __GIT_SHA__) in all
# site/*.html and site/modules/*.html files.
#
# Used by .github/workflows/deploy.yml in CI, and runnable locally for
# preview. Mutates files in place — in CI that's fine because each run
# starts from a clean checkout.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SITE_DIR="${REPO_ROOT}/site"
PARTIALS_DIR="${REPO_ROOT}/site-partials"
DOCS_DIR="${REPO_ROOT}/docs/modules"
MODULES_OUT="${SITE_DIR}/modules"

if [[ ! -d "${SITE_DIR}" ]]; then
  echo "error: ${SITE_DIR} not found" >&2
  exit 1
fi

# sed -i has different syntax on macOS vs GNU; use a portable wrapper
sed_inplace() {
  if [[ "$(uname)" == "Darwin" ]]; then
    sed -i "" "$@"
  else
    sed -i "$@"
  fi
}

# Substitute a placeholder line with the contents of a file.
# Uses awk so we don't have to escape every special character in the partial.
substitute_partial() {
  local placeholder="$1"
  local partial_file="$2"
  local target_file="$3"

  awk -v ph="${placeholder}" -v pf="${partial_file}" '
    $0 ~ ph {
      while ((getline line < pf) > 0) print line
      close(pf)
      next
    }
    { print }
  ' "${target_file}" > "${target_file}.tmp"
  mv "${target_file}.tmp" "${target_file}"
}

# =================================================================
# Phase 1: Generate per-module HTML pages from docs/modules/*.md
# =================================================================
if [[ -d "${DOCS_DIR}" ]]; then
  echo "Generating per-module pages from docs/modules/*.md"
  mkdir -p "${MODULES_OUT}"

  # Module page HTML template
  MODULE_TEMPLATE='<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Assay — __MODULE_TITLE__</title>
  <meta name="description" content="Assay module reference: __MODULE_TITLE__">
  <link rel="stylesheet" href="../style.css">
</head>
<body class="page-modules">
__HEADER__

  <main>
    <p><a href="../modules.html">&larr; All Modules</a></p>
__MODULE_CONTENT__
  </main>

__FOOTER__
</body>
</html>'

  # Module index entries (collected during generation)
  INDEX_ENTRIES=""

  for md_file in "${DOCS_DIR}"/*.md; do
    basename_noext="$(basename "${md_file}" .md)"

    # Extract title from first ## header
    title="$(head -1 "${md_file}" | sed 's/^## *//')"

    # Write HTML page from template
    echo "${MODULE_TEMPLATE}" \
      | sed "s|__MODULE_TITLE__|${title}|g" \
      > "${MODULES_OUT}/${basename_noext}.html"

    # Insert HTML body via file-based awk substitution (safe for large content)
    npx --yes marked "${md_file}" 2>/dev/null > /tmp/assay-module-body.html
    substitute_partial "__MODULE_CONTENT__" "/tmp/assay-module-body.html" "${MODULES_OUT}/${basename_noext}.html"

    # Collect index entry
    INDEX_ENTRIES="${INDEX_ENTRIES}        <li><a href=\"modules/${basename_noext}.html\">${title}</a></li>\n"
  done

  # ---------------------------------------------------------------
  # Generate modules.html index page
  # ---------------------------------------------------------------
  echo "Generating module index page"
  cat > "${SITE_DIR}/modules.html" <<INDEXEOF
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Assay — Module Reference</title>
  <meta name="description" content="Complete reference for all Assay modules — Rust builtins and embedded Lua stdlib modules, zero dependencies.">
  <link rel="stylesheet" href="style.css">
</head>
<body class="page-modules">
__HEADER__

  <main>
    <h1>Module Reference</h1>
    <p style="font-size: 1.15rem; color: var(--text-secondary); margin-bottom: 1.5rem;">
      All modules, zero dependencies. Use <code>assay context &lt;query&gt;</code> for LLM-ready docs on any module.
    </p>

    <pre><code>assay modules                    # list all modules
assay context "grafana health"   # get detailed docs for LLM</code></pre>

    <h2>Builtins (no require needed)</h2>
    <ul>
$(printf '%b' "${INDEX_ENTRIES}" | grep -E '(http|serialization|crypto|regex|db|ws|template|async|assert|utilities|fs)\.html' || true)
    </ul>

    <h2>Monitoring &amp; Observability</h2>
    <ul>
$(printf '%b' "${INDEX_ENTRIES}" | grep -E '(prometheus|alertmanager|loki|grafana)\.html' || true)
    </ul>

    <h2>Kubernetes &amp; GitOps</h2>
    <ul>
$(printf '%b' "${INDEX_ENTRIES}" | grep -E '(k8s|argocd|kargo|flux|traefik)\.html' || true)
    </ul>

    <h2>Security &amp; Identity</h2>
    <ul>
$(printf '%b' "${INDEX_ENTRIES}" | grep -E '(vault|openbao|certmanager|eso|dex|zitadel|ory)\.html' || true)
    </ul>

    <h2>Infrastructure</h2>
    <ul>
$(printf '%b' "${INDEX_ENTRIES}" | grep -E '(crossplane|velero|temporal|harbor)\.html' || true)
    </ul>

    <h2>Data &amp; Storage</h2>
    <ul>
$(printf '%b' "${INDEX_ENTRIES}" | grep -E '(postgres|s3)\.html' || true)
    </ul>

    <h2>Feature Flags &amp; Utilities</h2>
    <ul>
$(printf '%b' "${INDEX_ENTRIES}" | grep -E '(unleash|healthcheck)\.html' || true)
    </ul>

    <h2>AI Agent &amp; Workflow</h2>
    <ul>
$(printf '%b' "${INDEX_ENTRIES}" | grep -E '(ai-agents)\.html' || true)
    </ul>
  </main>

__FOOTER__
</body>
</html>
INDEXEOF

  # ---------------------------------------------------------------
  # Generate llms-full.txt from all markdown files
  # ---------------------------------------------------------------
  echo "Generating llms-full.txt from docs/modules/*.md"
  {
    # Header
    cat <<'LLMSHEADER'
# Assay

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

LLMSHEADER

    # Concatenate all module markdown files
    for md_file in "${DOCS_DIR}"/*.md; do
      cat "${md_file}"
      echo ""
      echo ""
    done

    # Footer
    cat <<'LLMSFOOTER'
## Optional
- [Crates.io](https://crates.io/crates/assay-lua): Use Assay as a Rust crate in your own projects
- [Docker](https://github.com/developerinlondon/assay/pkgs/container/assay): ghcr.io/developerinlondon/assay:latest (~9MB compressed)
- [Agent Guides](https://assay.rs/agent-guides.html): Integration guides for Claude Code, Cursor, Windsurf, Cline, OpenCode
- [Changelog](https://github.com/developerinlondon/assay/releases): Release history
LLMSFOOTER
  } > "${SITE_DIR}/llms-full.txt"

  echo "Generated $(ls "${MODULES_OUT}"/*.html 2>/dev/null | wc -l | tr -d ' ') module pages"
fi

# =================================================================
# Phase 2: Substitute partials into all HTML files
# =================================================================
echo "Substituting partials into site/*.html and site/modules/*.html"
html_files=("${SITE_DIR}"/*.html)
[[ -d "${MODULES_OUT}" ]] && html_files+=("${MODULES_OUT}"/*.html)
for f in "${html_files[@]}"; do
  [[ -f "$f" ]] || continue
  substitute_partial "__HEADER__" "${PARTIALS_DIR}/header.html" "${f}"
  substitute_partial "__FOOTER__" "${PARTIALS_DIR}/footer.html" "${f}"
done

# =================================================================
# Phase 3: Stamp version
# =================================================================
echo "Stamping version"
TAG="$(git -C "${REPO_ROOT}" describe --tags --exact-match HEAD 2>/dev/null || true)"
SHORT_SHA="$(git -C "${REPO_ROOT}" rev-parse --short HEAD 2>/dev/null || true)"
VERSION="${TAG:-${SHORT_SHA:-local-dev}}"
for f in "${html_files[@]}"; do
  [[ -f "$f" ]] || continue
  sed_inplace "s|__GIT_SHA__|${VERSION}|g" "${f}"
done

echo "Done. Site rendered at ${SITE_DIR}/"
