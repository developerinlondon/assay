#!/usr/bin/env bash
# Build script for the assay.rs static site.
#
# Substitutes the following placeholders in every site/*.html:
#   __HEADER__   → contents of site-partials/header.html
#   __FOOTER__   → contents of site-partials/footer.html
#   __GIT_SHA__  → current git tag or short SHA
#
# Used by .github/workflows/deploy.yml in CI, and runnable locally for
# preview. Mutates files in place — in CI that's fine because each run
# starts from a clean checkout. Locally, run on a clean working tree
# and `git checkout site/` afterwards if you want the placeholders back.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SITE_DIR="${REPO_ROOT}/site"
PARTIALS_DIR="${REPO_ROOT}/site-partials"

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

echo "Substituting partials into site/*.html"
for f in "${SITE_DIR}"/*.html; do
  substitute_partial "__HEADER__" "${PARTIALS_DIR}/header.html" "${f}"
  substitute_partial "__FOOTER__" "${PARTIALS_DIR}/footer.html" "${f}"
done

echo "Stamping version"
TAG="$(git -C "${REPO_ROOT}" describe --tags --exact-match HEAD 2>/dev/null || true)"
SHORT_SHA="$(git -C "${REPO_ROOT}" rev-parse --short HEAD 2>/dev/null || true)"
VERSION="${TAG:-${SHORT_SHA:-local-dev}}"
for f in "${SITE_DIR}"/*.html; do
  sed_inplace "s|__GIT_SHA__|${VERSION}|g" "${f}"
done

echo "Done. Site rendered at ${SITE_DIR}/"
