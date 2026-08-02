import { defineConfig, devices } from '@playwright/test';
import { phoneConfig } from '@xinutec/ui-harness/config';
import harness from './e2e/harness.mjs';

/**
 * Phone-width render checks for the console — NOT behavioural unit tests.
 *
 * This app has more claim on them than most: the phone is the reason it exists
 * (docs/agent-console.md, phase 3), and its two hardest layouts are hostile to a
 * 412px screen. A session's working directory is one long unbreakable path, and
 * a transcript is full of tool arguments that are paths and shell commands —
 * exactly the strings that push a page sideways. jsdom has no layout, so vitest
 * cannot see any of it.
 */
export default defineConfig(phoneConfig(harness, devices, { testMatch: '**/ui-pages.spec.ts' }));
