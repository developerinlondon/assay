import { defineConfig } from '@playwright/test';

// Playwright config for libs/hostops UI smoke. The hostops lib is
// booted on $E2E_PORT (default 47921) by run.sh — see that script for
// the boot flow. These specs only assert dashboard surfaces render
// and key data from the stub state shows up; deeper behaviour (form
// submissions, mutating endpoints) lives in the Lua-side tests.

export default defineConfig({
  testDir: '.',
  testMatch: ['**/*.spec.ts'],
  workers: 1,
  fullyParallel: false,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [['github'], ['list']] : [['list']],
  use: {
    baseURL: process.env.HOSTOPS_E2E_BASE || 'http://127.0.0.1:47921',
    headless: true,
    viewport: { width: 1400, height: 900 },
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
  },
  timeout: 30_000,
  expect: { timeout: 10_000 },
});
