import { test, expect, Page } from '@playwright/test';

const DEMO_WF_ID = process.env.ASSAY_E2E_WF_ID || 'demo-2';
const DEMO_NAMESPACE = process.env.ASSAY_E2E_NAMESPACE || 'demo';

async function openDashboard(page: Page) {
  await page.goto('/workflow/');
  // Switch to the demo namespace where the fixture worker registered.
  const sel = page.locator('select').first();
  await sel.waitFor({ state: 'visible' });
  await sel.selectOption(DEMO_NAMESPACE);
}

async function openDemoRow(page: Page) {
  const row = page.locator(`tr:has-text("${DEMO_WF_ID}")`).first();
  await row.waitFor({ state: 'visible' });
  await row.click();
}

test.describe('Steps tab', () => {
  test('is present and default-active when steps[] are emitted', async ({ page }) => {
    await openDashboard(page);
    await openDemoRow(page);

    const pipelineTabBtn = page.locator('.detail-tab[data-tab="pipeline"]').first();
    await expect(pipelineTabBtn).toBeVisible();
    await expect(pipelineTabBtn).toHaveClass(/active/);

    // Pipeline tab is FIRST: every other tab button comes after it.
    const tabIds = await page.locator('.detail-tab').evaluateAll(els =>
      els.map(e => (e as HTMLElement).dataset.tab)
    );
    expect(tabIds[0]).toBe('pipeline');
  });

  test('renders five circles with correct initial states + Approval has action buttons', async ({ page }) => {
    await openDashboard(page);
    await openDemoRow(page);

    const steps = page.locator('.pipeline-step');
    await expect(steps).toHaveCount(5);

    // Status text under each circle.
    await expect(steps.nth(0).locator('.step-status')).toHaveText('running');
    await expect(steps.nth(1).locator('.step-status')).toHaveText('waiting');
    await expect(steps.nth(4).locator('.step-status')).toHaveText('waiting');

    // Glyph + class for the running step. The running circle renders
    // an inline SVG spinner (not the unicode ⟳ arrow — rotating an
    // asymmetric glyph looks janky, a symmetric SVG ring rotates
    // cleanly).
    await expect(steps.nth(0)).toHaveClass(/running/);
    await expect(steps.nth(0).locator('.step-circle svg.step-spinner')).toHaveCount(1);

    // Action buttons render only on the Approval step.
    await expect(steps.nth(0).locator('.pipeline-step-action')).toHaveCount(2);
    await expect(steps.nth(1).locator('.pipeline-step-action')).toHaveCount(0);
  });

  test('step click filters the log to that step only', async ({ page }) => {
    await openDashboard(page);
    await openDemoRow(page);

    // Auto-advance pre-selects the running step (Approval). To test the
    // manual-click filter behaviour we drive it via a non-running step
    // (Tag & Retag at index 1, which is `waiting`) so the "select →
    // selected" transition isn't masked by auto-advance.
    const approval = page.locator('.pipeline-step').nth(0);
    await expect(approval).toHaveClass(/selected/); // sanity: auto-advanced

    const tag = page.locator('.pipeline-step').nth(1);
    await tag.click();
    await expect(tag).toHaveClass(/selected/);
    await expect(approval).not.toHaveClass(/selected/);

    // All log lines now visible should have data-step-idx="2".
    const visibleLines = page.locator('.pipeline-log-line:not(.hidden)');
    const count = await visibleLines.count();
    for (let i = 0; i < count; i++) {
      await expect(visibleLines.nth(i)).toHaveAttribute('data-step-idx', '2');
    }

    // Clicking the selected step again clears manual selection and
    // re-enables auto-advance.
    await tag.click();
    await expect(tag).not.toHaveClass(/selected/);
  });

  // Live-tail test runs LAST in this spec (Playwright preserves source
  // order with `fullyParallel: false`) because it mutates the demo
  // workflow's state — once Approval is approved, the buttons are gone
  // and the earlier "Approval has action buttons" assertion would fail
  // on a re-run within the same fixture instance.
  test('live tail: clicking Approve advances the pipeline', async ({ page }) => {
    await openDashboard(page);
    await openDemoRow(page);

    const approve = page.locator('.pipeline-step-action[data-action="approve"]').first();
    await expect(approve).toBeVisible();
    await approve.click();

    const approval = page.locator('.pipeline-step').nth(0);
    const tag = page.locator('.pipeline-step').nth(1);
    await expect(approval).toHaveClass(/done/);
    await expect(tag).toHaveClass(/running/);

    // Action buttons disappear from the now-completed Approval step.
    await expect(approval.locator('.pipeline-step-action')).toHaveCount(0);

    // Connector between Approval and Tag is now partial (done → running).
    await expect(page.locator('.pipeline-connector').nth(0)).toHaveClass(/partial/);

    // A new log line should mention the next step kicking off.
    await expect(page.locator('.pipeline-log-line').last()).toContainText(/Tag/);
  });
});
