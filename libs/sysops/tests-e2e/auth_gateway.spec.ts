// End-to-end test against the live gondor server on :18790, exercising
// the 0.2.0 sysops auth gateway. Assumes:
//   - gondor.service is running locally (see /etc/systemd/system/gondor.service)
//   - GONDOR_E2E_COOKIE is a valid gondor_session cookie value
//   - GONDOR_E2E_BASE is http://127.0.0.1:18790 (or override via env)
//
// Cookie is injected via an env var so the spec doesn't have to know how
// to mint one. Use the run-it-now harness (scripts/run-auth-gateway-e2e.sh)
// to set this up.

import { expect, test } from '@playwright/test';

const BASE   = process.env.GONDOR_E2E_BASE   || 'http://127.0.0.1:18790';
const COOKIE = process.env.GONDOR_E2E_COOKIE || '';

if (!COOKIE) throw new Error('GONDOR_E2E_COOKIE env var required');

test.use({ baseURL: BASE });

test.beforeEach(async ({ context }) => {
  await context.addCookies([{
    name:     'gondor_session',
    value:    COOKIE,
    url:      BASE,
    httpOnly: true,
    sameSite: 'Lax',
  }]);
});

test('dashboard renders and the Engine link is same-origin (not localhost)', async ({ page }) => {
  const response = await page.goto('/');
  expect(response?.status()).toBe(200);

  const engineLink = page.locator('a:has-text("Engine")').first();
  await expect(engineLink).toBeVisible();
  const href = await engineLink.getAttribute('href');
  expect(href).toBe('/engine/console');
});

test('clicking Engine reaches the engine SPA HTML via the proxy', async ({ page }) => {
  const response = await page.goto('/engine/console');
  expect(response?.status()).toBe(200);

  // Engine SPA shells declare data-default-theme on <html>.
  await expect(page.locator('html[data-default-theme]')).toBeVisible();

  // The actual token-banner is .auth-token-banner — only injected
  // when both adminToken and hasSession are false. With the /whoami
  // intercept active server-side, the banner is never built. (We don't
  // assert on the #admin-text status pill — its post-boot value depends
  // on the SPA's in-browser fetch including the session cookie, which
  // is Playwright-context-sensitive. The server-side intercept itself
  // is verified by the '/whoami intercept' test below.)
  await expect(page.locator('.auth-token-banner')).toHaveCount(0);
});

test('workflow SPA loads and version API returns 200 via proxy', async ({ page, request }) => {
  // workflow SPA opens an SSE stream (/api/v1/engine/workflow/events/stream)
  // at boot which the lua proxy can't relay (http.get is buffered, not
  // streaming). The page never reaches networkidle. Just wait for the
  // DOM to settle and assert the SPA HTML loaded.
  await page.goto('/workflow', { waitUntil: 'domcontentloaded' });
  await expect(page.locator('html[data-default-theme]')).toBeVisible();

  // workflow SPA has no token banner (it's silent — sends bearer if
  // available, falls back to cookies).
  const versionResp = await request.get(`${BASE}/api/v1/engine/workflow/version`, {
    headers: { Cookie: `gondor_session=${COOKIE}` },
  });
  expect(versionResp.status()).toBe(200);
  const body = await versionResp.json();
  expect(body).toHaveProperty('version');
});

test('/whoami intercept returns the session identity', async ({ request }) => {
  const r = await request.get(`${BASE}/api/v1/engine/auth/whoami`, {
    headers: { Cookie: `gondor_session=${COOKIE}` },
  });
  expect(r.status()).toBe(200);
  const body = await r.json();
  expect(body).toHaveProperty('sub');
  expect(body).toHaveProperty('email');
});

test('engine-admin API call is proxied with admin-bearer injection', async ({ request }) => {
  // The engine's /api/v1/engine/core/info endpoint is admin-only on the
  // engine side. With the session cookie we should reach it via the
  // proxy and get the engine version metadata.
  const r = await request.get(`${BASE}/api/v1/engine/core/info`, {
    headers: { Cookie: `gondor_session=${COOKIE}` },
  });
  expect(r.status()).toBe(200);
  const body = await r.json();
  expect(body).toHaveProperty('version');
  expect(body).toHaveProperty('instance_id');
});

test('no cookie → /api/v1/engine/* is 401', async ({ request }) => {
  const r = await request.get(`${BASE}/api/v1/engine/core/info`);
  expect(r.status()).toBe(401);
});

test('unauthenticated visit to / 302s to /auth/login', async ({ page, context }) => {
  await context.clearCookies();
  const response = await page.goto('/', { waitUntil: 'commit' });
  // After the 302, the URL should be /auth/login or further along the chain.
  // We allow either /auth/login or the IdP host (if the IdP doesn't have
  // a session of its own and we get redirected further).
  const url = response?.url() || page.url();
  expect(url).toMatch(/\/auth\/(login|authorize|callback)|gondor-engine/);
});
