import { defineConfig, devices } from '@playwright/test';

/**
 * The two-device sync suite — NOT the phone-width harness.
 *
 * Its own config because it needs the opposite of that one: no static server and
 * no stubbed API, because the whole point is a REAL runner serving both the app
 * and `/api/sync/drafts`. `e2e/runner.mjs` starts it, so there is no `webServer`
 * here either — the fixture has to write a transcript before the runner reads
 * the directory, which a command line cannot express.
 *
 * Serial, and one worker: the tests share one runner process, and a draft is
 * state on it.
 */
export default defineConfig({
  testDir: './e2e',
  testMatch: '**/two-devices.spec.ts',
  fullyParallel: false,
  workers: 1,
  // Sync is a poll, so every assertion here is "within the interval" rather than
  // "now". The per-expect timeouts say so; this only has to outlast them.
  timeout: 120_000,
  reporter: [['list']],
  // Named rather than defaulted, so the gate's `artifacts` path and this agree.
  // The gate reported "the check wrote nothing there" on a real failure.
  outputDir: './test-results',
  use: { ...devices['Desktop Chrome'] },
});
