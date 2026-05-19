# Putting `assay-engine` behind Cloudflare Access

Two viable patterns, both covered below. **TL;DR**: a single bypass on the whole host (Rauthy/Dex
pattern) is **wrong** for assay-engine because the host serves more than OIDC. Use path-scoped
policies.

## Why assay-engine differs from a standalone IdP

```
auth.fcar.ai (Rauthy)                  gondor-engine.fcar.ai (assay-engine)
─────────────────────────────          ─────────────────────────────────────
/.well-known/*  OIDC spec              /auth/.well-known/*  OIDC spec
/auth/*         OIDC + login UI        /auth/*              OIDC + login UI
                                       /auth/console        ← admin SPA
                                       /api/v1/engine/auth/admin/*
                                                            ← admin API
                                       /api/v1/vault/*      ← vault data API
                                       /api/v1/engine/workflow/*
                                                            ← workflow API
                                       /vault/, /workflow/,
                                        /engine/console     ← admin SPAs
```

Rauthy's host serves only OIDC-spec + its own login. Whole-host bypass exposes nothing sensitive.
assay-engine's host serves vault data, workflow control, multiple admin SPAs. Whole-host bypass
exposes all of those.

## Choice 1 (recommended) — no CF Access on the engine host

```
gondor.example.com           CF Access  (email allow-list)        Layer 1
                            └─ dashboard does its own login via
                               assay-auth session (Layer 2)
                            
gondor-engine.example.com    (NO CF Access)
                            └─ engine self-protects:
                               - SPA shells: session-gate middleware
                                 redirects unauth to /auth/login
                               - admin routes: admin_api_keys bearer
                                 OR session+zanzibar via require_role_for
                               - OIDC spec endpoints: public-by-spec
                               - vault biscuit / workflow zanzibar:
                                 module-level auth
```

This matches the Rauthy/Dex pattern in spirit — the IdP guards itself. The complexity of path-scoped
CF Access policies disappears.

## Choice 2 — defense-in-depth (CF Access in front)

Use ONLY if you specifically want a second perimeter layer.

```
Two Access apps on the same hostname, evaluated by path specificity:

  app A:  gondor-engine.example.com           email allow-list (catch-all)
  app B:  gondor-engine.example.com/auth      bypass (everyone)
  app C:  gondor-engine.example.com/auth/console
                                              email allow-list (overrides B)

CF Access picks the most specific path match:
  /auth/.well-known/openid-configuration   → app B (bypass)
  /auth/token                              → app B (bypass)   ← server-to-server token exchange
  /auth/login                              → app B (bypass)
  /auth/oidc/upstream/google/callback      → app B (bypass)
  /auth/console                            → app C (allow-list)  ← admin SPA stays gated
  /api/v1/engine/auth/admin/*              → app A (allow-list)
  /api/v1/vault/*                          → app A (allow-list)
  /api/v1/engine/workflow/*                → app A (allow-list)
  /vault/, /workflow/, /engine/console     → app A (allow-list)
```

### What MUST be bypassed (any OIDC deployment)

These endpoints are public-by-OIDC-spec — server-to-server callers hit them without a browser-scoped
CF Access cookie, so an email-PIN allow-list will black-hole them:

```
/auth/.well-known/openid-configuration      RFC 8414 — RP discovery
/auth/.well-known/jwks.json                 RFC 7517 — sig verification
/auth/token                                 OIDC Core §3.1.3
/auth/userinfo                              OIDC Core §5.3
/auth/oidc/upstream/{slug}/callback         federation return
```

The simplest correct scope is `domain: <host>/auth` + `decision: bypass`.

### What MUST stay gated

Everything that's NOT public-by-spec. Cover them with the catch-all app A and the more-specific
override C:

```
/api/v1/engine/auth/admin/*       admin user/session/oidc/zanzibar CRUD
/api/v1/vault/*                   vault data
/api/v1/engine/workflow/*         workflow control
/vault/, /workflow/, /engine/console, /auth/console
                                  admin SPA shells
```

## Pitfalls observed

1. **Whole-host bypass.** Mirrors Rauthy literally — exposes vault data API. Use path-scoped
   instead.
2. **No `/auth/*` bypass at all.** Server-to-server token exchange from the dashboard 302s to a CF
   Access login HTML page, dashboard tries `json.parse` on HTML, crashes. Symptom seen in the field:
   `runtime error: json.parse: expected value at line 1 column 1`.
3. **Subdomain cookie scope.** CF Access cookies are per-subdomain by default. A user who
   authenticates to `gondor.example.com` still hits a second email-PIN screen on
   `gondor-engine.example.com` unless you configure an IdP (Google/etc.) in Zero Trust so the org
   token survives across apps. With per-subdomain PIN you'll see two prompts.
4. **OIDC RPs outside the box.** Adding CF Access to the engine even with the bypass is fine for
   browser-led OIDC flows from `gondor.example.com`'s dashboard. RPs that aren't co-located (a k3s
   pod elsewhere, another engine, etc.) need either their own CF Access service token credentials OR
   the engine to be reachable without CF Access at all.

## Decision rule

```
single-tenant, single host, only browser RPs   → Choice 2 path-scoped
multi-host, off-box RPs, ops simplicity         → Choice 1 no CF Access
```

When in doubt, **Choice 1**. The engine has its own auth front-to-back once invite-only + bootstrap
admin + session-gate are wired (see `zanzibar-namespace-seed.md`).
