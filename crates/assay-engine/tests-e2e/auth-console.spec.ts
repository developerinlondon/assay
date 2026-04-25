import { test, expect, Page } from '@playwright/test';
import { withAdminToken, waitForCrossNav } from './_setup';

test.describe('Auth console panes', () => {
  test('shell loads + sidebar pills navigate without errors', async ({ page }) => {
    await withAdminToken(page, '/auth/console');
    // Each named pane should render its h2/h3 within ~10s.
    const panes: { view: string; expectText: RegExp }[] = [
      { view: 'users',         expectText: /Users/i },
      { view: 'sessions',      expectText: /Sessions/i },
      { view: 'oidc-clients',  expectText: /Clients|OIDC/i },
      { view: 'oidc-upstream', expectText: /Upstream|Provider/i },
      { view: 'zanzibar',      expectText: /Zanzibar|Tuples|Namespace/i },
      { view: 'keys',          expectText: /Biscuit|JWKS|Key/i },
      { view: 'audit',         expectText: /Audit/i },
    ];
    for (const p of panes) {
      const link = page.locator(`.nav-link[data-view="${p.view}"]`);
      await link.click();
      // Either a section title or an empty-state — never a hard error
      // banner. Wait for either to land.
      await page.locator('main h2, main h3, .auth-empty').first().waitFor({ state: 'visible' });
      const text = await page.locator('main').innerText();
      // Soft-assert against the per-pane substring so we know the
      // right component painted.
      expect(text).toMatch(p.expectText);
    }
  });

  test('Users pane shows seeded fixtures', async ({ page }) => {
    await withAdminToken(page, '/auth/console');
    await page.locator('.nav-link[data-view="users"]').click();
    await page.locator('table.data-table').waitFor({ state: 'visible' });
    const body = page.locator('table.data-table tbody');
    // alice + admin are minted by seed-sample.
    await expect(body).toContainText('alice@example.com');
    await expect(body).toContainText('admin@example.com');
  });

  test('Users round-trip: create + see + delete', async ({ page }) => {
    // Drive the round-trip via the SPA UI exclusively so we exercise
    // the form + table refresh + delete confirm flows end to end.
    await withAdminToken(page, '/auth/console');
    await page.locator('.nav-link[data-view="users"]').click();
    await page.locator('button#users-new').click();

    const email = `e2e-${Date.now()}@example.test`;
    await page.locator('input#nu-email').fill(email);
    await page.locator('input#nu-name').fill('E2E Roundtrip');
    await page.locator('input#nu-pw').fill('roundtrip-pw');
    await page.locator('button#nu-create').click();

    // Wait for the table to come back with the new row visible.
    const row = page.locator(`tr:has-text("${email}")`);
    await row.waitFor({ state: 'visible' });

    // Delete via the row's Delete button. The SPA prompts via
    // window.confirm — auto-accept it via the dialog handler.
    page.once('dialog', (d) => d.accept());
    await row.locator('button[data-action="delete"]').click();

    // Row should be gone after the table reloads.
    await expect(page.locator(`tr:has-text("${email}")`)).toHaveCount(0, { timeout: 15_000 });
  });

  test('Cross-nav highlights the Auth pill', async ({ page }) => {
    await withAdminToken(page, '/auth/console');
    await waitForCrossNav(page);
    await expect(page.locator('.cross-nav-pill[data-pill="auth"]')).toHaveClass(/active/);
  });
});

async function _unused(_p: Page) {
  // Keep the Page import valid for future helpers without fighting
  // the unused-import lint.
}
