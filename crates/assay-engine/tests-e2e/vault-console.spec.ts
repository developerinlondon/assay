import { test, expect } from '@playwright/test';
import { waitForCrossNav } from './_setup';

test.describe('Vault console', () => {
  test('chrome sits below the fixed cross-console header', async ({ page }) => {
    await page.goto('/workflow/');
    await page.evaluate((token) => {
      window.localStorage.setItem('assay-admin-token', token);
      window.localStorage.setItem('assay-theme', 'light');
    }, process.env.ASSAY_E2E_ADMIN_KEY || 'dev-admin-key-change-me');
    await page.goto('/vault/console');
    await waitForCrossNav(page);
    await expect(page.locator('.console-tab[data-tab="vault"]')).toHaveClass(/active/);
    await expect(page.locator('#cross-nav-version')).toHaveText(/v\d+\.\d+\.\d+/);
    await expect(page.locator('#cross-nav-leader-dot')).toHaveClass(/leader/);
    await expect(page.locator('#cross-nav-instance')).toContainText(/instance:/);
    await expect(page.locator('#cross-nav-instance')).not.toHaveText('instance:—');

    const layout = await page.evaluate(() => {
      const header = document.querySelector('.cross-nav')?.getBoundingClientRect();
      const sidebarHeader = document.querySelector('.sidebar-header')?.getBoundingClientRect();
      const main = document.querySelector('.main-content')?.getBoundingClientRect();
      if (!header || !sidebarHeader || !main) {
        throw new Error('vault console chrome was not rendered');
      }
      return {
        headerBottom: header.bottom,
        sidebarHeaderTop: sidebarHeader.top,
        mainTop: main.top,
      };
    });

    expect(layout.sidebarHeaderTop).toBeGreaterThanOrEqual(layout.headerBottom);
    expect(layout.mainTop).toBeGreaterThanOrEqual(layout.headerBottom);

    const colors = await page.evaluate(() => {
      const probe = document.createElement('div');
      probe.style.backgroundColor = 'var(--surface)';
      document.body.appendChild(probe);
      const surface = getComputedStyle(probe).backgroundColor;
      probe.remove();

      const card = document.querySelector('.pane-status .card');
      const button = document.querySelector('#btn-init');
      if (!card || !button) {
        throw new Error('vault sealing controls were not rendered');
      }
      return {
        surface,
        cardBackground: getComputedStyle(card).backgroundColor,
        buttonBackground: getComputedStyle(button).backgroundColor,
      };
    });

    expect(colors.cardBackground).toBe(colors.surface);
    expect(colors.buttonBackground).toBe(colors.surface);
  });
});
