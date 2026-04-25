import { defineConfig } from '@playwright/test';

// E2E tests for the assay-engine + auth consoles + cross-console nav.
// The engine (with auth + admin api-keys configured) must already be
// running on http://localhost:8420 before invoking these tests. In CI
// that's done by the `e2e-engine` job in .github/workflows/ci.yml; for
// local runs see fixtures/README.md in this directory.
export default defineConfig({
  testDir: '.',
  testMatch: ['**/*.spec.ts'],
  // Single worker so the seed-sample fixtures land deterministically
  // and tests that mutate state (e.g. user create/delete in
  // auth-console.spec.ts) run serially.
  workers: 1,
  fullyParallel: false,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [['github'], ['list']] : [['list']],
  use: {
    baseURL: process.env.ASSAY_E2E_BASE || 'http://localhost:8420',
    headless: true,
    viewport: { width: 1400, height: 900 },
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
    extraHTTPHeaders: {
      // Threaded into request.* helpers so /api/v1/engine/* admin reads
      // work without the SPA token banner. UI flows separately persist
      // the same token via window.localStorage in the spec setup.
      'authorization': 'Bearer ' + (process.env.ASSAY_E2E_ADMIN_KEY || 'dev-admin-key-change-me'),
    },
  },
  timeout: 30_000,
  expect: { timeout: 10_000 },
});
