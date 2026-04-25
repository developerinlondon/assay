import { Page } from '@playwright/test';

/**
 * Persist the admin token in localStorage before navigating to a
 * console route so the SPA's auth-banner-on-empty-token branch
 * doesn't fire and we land directly on the requested pane.
 *
 * Both the auth and engine consoles read `assay-admin-token` from
 * window.localStorage on init.
 */
export async function withAdminToken(page: Page, route: string): Promise<void> {
  const token = process.env.ASSAY_E2E_ADMIN_KEY || 'dev-admin-key-change-me';
  // The localStorage call has to happen on the same origin we're
  // navigating to. Visit a tiny path first (the asset 404 page is
  // fine — origin is still ours) then set the key, then navigate.
  // The /workflow/ shell is the safest bootstrap target since the
  // workflow module is always enabled in the e2e fixture config.
  await page.goto('/workflow/');
  await page.evaluate((t) => {
    window.localStorage.setItem('assay-admin-token', t);
  }, token);
  await page.goto(route);
}

/**
 * Wait for the cross-nav strip to be populated (it's filled in by an
 * async fetch of /api/v1/modules + /api/v1/engine/info).
 */
export async function waitForCrossNav(page: Page): Promise<void> {
  await page.locator('.cross-nav-pill').first().waitFor({ state: 'visible' });
}
