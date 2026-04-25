import { test, expect } from '@playwright/test';
import { withAdminToken, waitForCrossNav } from './_setup';

test.describe('Engine console', () => {
  test('Info pane populates from /api/v1/engine/core/info', async ({ page }) => {
    await withAdminToken(page, '/engine/console');

    // Section title sanity.
    await expect(page.locator('main h2.section-title').first()).toHaveText('Engine');

    // Info cards render — six fixed labels from the wireframe.
    const labels = await page.locator('.engine-info-card .label').allTextContents();
    expect(labels).toEqual(
      expect.arrayContaining(['Version', 'Instance ID', 'Started', 'Uptime', 'Backend', 'Modules'])
    );

    // Version card has a non-empty `v…` value.
    const versionCard = page.locator('.engine-info-card', { hasText: 'Version' }).first();
    await expect(versionCard.locator('.value')).toContainText(/^v\d+\.\d+/);
  });

  test('Modules table loads and includes the workflow + auth rows', async ({ page }) => {
    await withAdminToken(page, '/engine/console');
    await page.locator('.nav-link[data-view="modules"]').click();

    // Wait for the table to render.
    await page.locator('table.data-table').waitFor({ state: 'visible' });
    const rows = page.locator('table.data-table tbody tr');
    await expect(rows).not.toHaveCount(0);

    // Workflow + auth modules are seeded by the engine boot path.
    await expect(page.locator('table.data-table tbody')).toContainText('workflow');
    await expect(page.locator('table.data-table tbody')).toContainText('auth');
  });

  test('Instances pane lists at least one row', async ({ page }) => {
    await withAdminToken(page, '/engine/console');
    await page.locator('.nav-link[data-view="instances"]').click();
    await page.locator('table.data-table').waitFor({ state: 'visible' });
    const rows = page.locator('table.data-table tbody tr');
    expect(await rows.count()).toBeGreaterThanOrEqual(1);
  });

  test('Audit pane paginates without errors', async ({ page }) => {
    await withAdminToken(page, '/engine/console');
    await page.locator('.nav-link[data-view="audit"]').click();
    // Either rows or the "no rows match" empty state — both are valid
    // depending on whether seed-sample triggered any audit-worthy
    // events. Wait for one of them to render so we know the page
    // didn't error out.
    await page.locator('table.data-table, .auth-empty').first().waitFor({ state: 'visible' });
  });

  test('Config pane renders the JSON view with admin_api_keys redacted', async ({ page }) => {
    await withAdminToken(page, '/engine/console');
    await page.locator('.nav-link[data-view="config"]').click();
    const pre = page.locator('pre.engine-config-pre');
    await pre.waitFor({ state: 'visible' });
    const text = await pre.textContent();
    expect(text).toBeTruthy();
    // [REDACTED] placeholder in place of the configured admin api-key
    // — must NEVER expose dev-admin-key-change-me on the dashboard.
    expect(text).toContain('[REDACTED]');
    expect(text).not.toContain('dev-admin-key-change-me');
  });

  test('Cross-nav strip renders + Engine pill is active', async ({ page }) => {
    await withAdminToken(page, '/engine/console');
    await waitForCrossNav(page);
    const enginePill = page.locator('.cross-nav-pill[data-pill="engine"]');
    await expect(enginePill).toBeVisible();
    await expect(enginePill).toHaveClass(/active/);
  });
});
