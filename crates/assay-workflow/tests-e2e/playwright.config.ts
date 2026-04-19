import { defineConfig } from '@playwright/test';

// E2E tests for the assay-workflow dashboard. The engine + a demo
// worker that emits the canonical pipeline_state shape must already
// be running on http://localhost:8080 before invoking these tests.
// In CI that's done by the `e2e` job in .github/workflows/ci.yml; for
// local runs see fixtures/README.md in this directory.
export default defineConfig({
  testDir: '.',
  testMatch: ['**/*.spec.ts'],
  // Single worker so tests share the one demo workflow and run in a
  // deterministic order (the live-tail test mutates state by sending a
  // step_action signal, and later tests assume the workflow hasn't yet
  // advanced past Approval).
  workers: 1,
  fullyParallel: false,
  // CI runs are flaky-tolerant on the live-tail test by design (1 Hz
  // poll + ~3 s pipeline tick). One retry buys us robustness without
  // hiding genuine regressions: a real bug fails twice, intermittent
  // poll timing fails once.
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [['github'], ['list']] : [['list']],
  use: {
    baseURL: process.env.ASSAY_E2E_BASE || 'http://localhost:8080',
    headless: true,
    viewport: { width: 1400, height: 900 },
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
  },
  timeout: 30_000,
  expect: { timeout: 10_000 },
});
