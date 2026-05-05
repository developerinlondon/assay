# 23 · engine SPA matches sysops design system

**Status:** spec\
**Date:** 2026-05-03

## Goal

`crates/assay-dashboard/` SPA visually continuous with sysops — same fonts, surfaces, button styles,
type ramp. Today only token-level whitelabel (NAME, MARK, accent) applies via
`ASSAY_WHITELABEL_CSS_URL`; layout/components still feel like a different product.

```mermaid
flowchart LR
  subgraph current["today"]
    h1[sysops UI<br/>dark, IBM Plex/JetBrains Mono]
    e1[engine SPA<br/>light, system fonts]
    h1 -.brand pack tokens.-> e1
  end
  subgraph target["after plan 23"]
    shared[shared design tokens<br/>colors, fonts, radius, type ramp]
    h2[sysops UI] -.uses.-> shared
    e2[engine SPA] -.uses.-> shared
  end
```

## Approach

Adopt sysops's tokens as the canonical assay design system. Engine SPA's `tokens.css` gets rewritten
to match sysops's. Component CSS continues to live where it does — only the variable definitions
change.

| File                                                                    | Change                                                                         |
| ----------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| `libs/sysops/static/css/tokens.css`                                     | extracted to `crates/assay-dashboard/assets/shared/tokens.css` (single source) |
| `crates/assay-dashboard/assets/{auth,vault,workflow,engine}/index.html` | `<link>` shared tokens.css before per-console style.css                        |
| `libs/sysops/templates/layout.html`                                     | `<link>` shared tokens.css instead of its own copy                             |
| `crates/assay-dashboard/assets/{auth,vault,workflow,engine}/style.css`  | re-tune button/surface rules to use the shared tokens (most already do)        |
| `crates/assay-dashboard/src/whitelabel.rs`                              | document the new shared-tokens convention                                      |

## What stays out

- Sidebar nav structure differs intentionally (engine SPA has cross-console pills, sysops has
  top-level nav). Plan 23 unifies tokens, not layout primitives.
- Component-level rewrites (form inputs, modals) remain on each side.

## Validation

- Render every engine SPA page side-by-side with the corresponding sysops page in Playwright; assert
  tokens (`getComputedStyle(el).color`, `font-family`) match for headings / body / muted / accent.
- Visual diff against current screenshots; brand pack overlay unchanged.

## Delivery

One assay PR. Bumps `assay-dashboard 0.3.0 → 0.4.0` (asset version), rolls into the next sysops
release.

## Open

1. Engine SPA's existing visual identity vs sysops — which side adapts? Default: engine adopts
   sysops (the host-ops dashboard is the entry point; engine is back-office).
2. Light-theme support — sysops dark-only today. Engine SPA has both. If shared tokens are adopted,
   do we extend sysops to dual-theme too?
