import { defineConfig, devices } from '@playwright/test';

/**
 * Playwright UI-render checks — NOT behavioural unit tests. They render the app
 * in a real browser at true phone geometry and assert measurable facts about
 * the pixels (icon fonts actually load; no text overlaps; nothing spills past
 * the right edge). jsdom has no fonts or layout, so a mat-icon that falls back
 * to its ligature word ("search") reads green in vitest and only the render
 * disagrees. Shared checkers live in @xinutec/ui-harness (repo ~/Code/ui-harness);
 * see dev-lint/docs/layout-quality-architecture.md.
 *
 * Runs against the PRODUCTION build served statically by e2e/serve.mjs — one
 * device, identical to messages/life/fleetwatch. `npm run ui-check` (wired into
 * scripts/verify.sh after `ng build`) serves the freshly-built dist;
 * reuseExistingServer attaches to a serve.mjs already up.
 */
// Unique across the fleet. Every app's harness sets `reuseExistingServer: true`,
// so two apps sharing a port silently attach to each OTHER's server and both
// suites fail in ways that never reproduce alone. This was 4273, which is
// health's.
const PORT = 4275;

export default defineConfig({
  testDir: './e2e',
  testMatch: '**/ui-pages.spec.ts',
  reporter: [['list']],
  timeout: 90_000,
  use: {
    baseURL: `http://localhost:${PORT}`,
    screenshot: 'only-on-failure',
  },
  // Emulate the phone this is actually read on (Pixel 9 ≈ the Pixel 7 preset:
  // 412 CSS px, mobile UA, touch). The viewport MUST live in the PROJECT `use`,
  // not the global one: a device spread carries its own viewport and
  // project-level `use` wins — that's how a "phone width" suite elsewhere
  // silently ran at desktop width. deviceScaleFactor is forced to 1 so CSS-pixel
  // geometry (what the layout checks measure) is DPR-invariant.
  projects: [{ name: 'chromium', use: { ...devices['Pixel 7'], deviceScaleFactor: 1 } }],
  webServer: {
    command: `node e2e/serve.mjs ${PORT}`,
    url: `http://localhost:${PORT}/`,
    reuseExistingServer: true,
    timeout: 60_000,
  },
});
