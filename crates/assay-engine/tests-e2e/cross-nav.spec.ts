import { test, expect } from '@playwright/test';
import { withAdminToken, waitForCrossNav } from './_setup';

test.describe('Cross-console nav strip', () => {
  test('all three pills render on the workflow console', async ({ page }) => {
    await withAdminToken(page, '/workflow/');
    await waitForCrossNav(page);
    await expect(page.locator('.cross-nav-pill[data-pill="workflow"]')).toBeVisible();
    await expect(page.locator('.cross-nav-pill[data-pill="auth"]')).toBeVisible();
    await expect(page.locator('.cross-nav-pill[data-pill="engine"]')).toBeVisible();
    await expect(page.locator('.cross-nav-pill[data-pill="workflow"]')).toHaveClass(/active/);
  });

  test('all three pills render on the auth console', async ({ page }) => {
    await withAdminToken(page, '/auth/console');
    await waitForCrossNav(page);
    await expect(page.locator('.cross-nav-pill[data-pill="workflow"]')).toBeVisible();
    await expect(page.locator('.cross-nav-pill[data-pill="auth"]')).toBeVisible();
    await expect(page.locator('.cross-nav-pill[data-pill="engine"]')).toBeVisible();
    await expect(page.locator('.cross-nav-pill[data-pill="auth"]')).toHaveClass(/active/);
  });

  test('all three pills render on the engine console', async ({ page }) => {
    await withAdminToken(page, '/engine/console');
    await waitForCrossNav(page);
    await expect(page.locator('.cross-nav-pill[data-pill="workflow"]')).toBeVisible();
    await expect(page.locator('.cross-nav-pill[data-pill="auth"]')).toBeVisible();
    await expect(page.locator('.cross-nav-pill[data-pill="engine"]')).toBeVisible();
    await expect(page.locator('.cross-nav-pill[data-pill="engine"]')).toHaveClass(/active/);
  });

  test('clicking a pill navigates between consoles', async ({ page }) => {
    await withAdminToken(page, '/workflow/');
    await waitForCrossNav(page);

    await page.locator('.cross-nav-pill[data-pill="auth"]').click();
    await expect(page).toHaveURL(/\/auth\/console/);
    await waitForCrossNav(page);
    await expect(page.locator('.cross-nav-pill[data-pill="auth"]')).toHaveClass(/active/);

    await page.locator('.cross-nav-pill[data-pill="engine"]').click();
    await expect(page).toHaveURL(/\/engine\/console/);
    await waitForCrossNav(page);
    await expect(page.locator('.cross-nav-pill[data-pill="engine"]')).toHaveClass(/active/);

    await page.locator('.cross-nav-pill[data-pill="workflow"]').click();
    await expect(page).toHaveURL(/\/workflow/);
  });

  test('header bar identity strip populates with version + leader', async ({ page }) => {
    await withAdminToken(page, '/engine/console');
    // The version span gets filled from /api/v1/engine/info — wait
    // until the placeholder ("v—") flips to a real semver string.
    await expect(page.locator('#cross-nav-version')).toHaveText(/v\d+\.\d+\.\d+/, { timeout: 10_000 });
    // Leader dot toggles `.leader` when info.leader=true.
    await expect(page.locator('#cross-nav-leader-dot')).toHaveClass(/leader/);
    // Instance string rolls in (truncated UUID head…tail).
    await expect(page.locator('#cross-nav-instance')).toContainText(/instance:/);
    await expect(page.locator('#cross-nav-instance')).not.toHaveText('instance:—');
  });
});
