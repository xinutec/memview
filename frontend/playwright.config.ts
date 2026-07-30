import { defineConfig, devices } from '@playwright/test';
import { phoneConfig } from '@xinutec/ui-harness/config';
import harness from './e2e/harness.mjs';

/**
 * Playwright UI-render checks — NOT behavioural unit tests. They render the app
 * in a real browser at true phone geometry and assert measurable facts about
 * the pixels (icon fonts actually load; no text overlaps; nothing spills past
 * the right edge). jsdom has no fonts or layout, so a mat-icon that falls back
 * to its ligature word ("search") reads green in vitest and only the render
 * disagrees.
 *
 * Everything shared — the Pixel geometry, the port, the static server serving
 * the PRODUCTION build — comes from @xinutec/ui-harness (repo ~/Code/ui-harness);
 * see dev-lint/docs/layout-quality-architecture.md. What this app says about
 * itself is in e2e/harness.mjs.
 *
 * `npm run ui-check` (wired into scripts/verify.sh after `ng build`) serves the
 * freshly-built dist.
 */
export default defineConfig(phoneConfig(harness, devices, { testMatch: '**/ui-pages.spec.ts' }));
