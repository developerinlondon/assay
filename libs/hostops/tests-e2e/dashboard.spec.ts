import { test, expect } from '@playwright/test';

test.describe('hostops dashboard', () => {
  test('top-level page renders sidebar + brand + host name', async ({ page }) => {
    await page.goto('/');
    await expect(page).toHaveTitle(/Test/);
    // Brand block appears in the layout sidebar.
    await expect(page.locator('aside .brand')).toBeVisible();
    await expect(page.locator('aside .brand-name')).toContainText('Test Brand');
    await expect(page.locator('aside .brand-host')).toContainText('test-host');
  });

  test('sidebar lists every host-ops section', async ({ page }) => {
    await page.goto('/');
    const nav = page.locator('aside .nav.nav-flat a');
    const labels = await nav.allTextContents();
    expect(labels).toEqual(
      expect.arrayContaining(['Host', 'nspawn containers', 'Networks', 'Admin'])
    );
  });

  test('static asset /static/styles.css served as text/css', async ({ request }) => {
    const r = await request.get('/static/styles.css');
    expect(r.status()).toBe(200);
    expect(r.headers()['content-type']).toContain('text/css');
  });
});

test.describe('hostops machines', () => {
  test('/machines lists every fixture machine', async ({ page }) => {
    await page.goto('/machines');
    await expect(page.locator('main')).toContainText('agentx');
    await expect(page.locator('main')).toContainText('k3s-server');
  });
});

test.describe('hostops sub-pages render without 5xx', () => {
  for (const path of ['/services', '/cron', '/logs', '/tunnels', '/tailscale', '/interfaces', '/audit', '/backups']) {
    test(`GET ${path} returns 200 + sidebar`, async ({ page, request }) => {
      const r = await request.get(path);
      expect(r.status()).toBe(200);
      await page.goto(path);
      await expect(page.locator('aside')).toBeVisible();
    });
  }
});
