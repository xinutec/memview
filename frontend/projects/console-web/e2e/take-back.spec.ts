import { test, expect, type Browser, type BrowserContext, type Page } from '@playwright/test';

import { start, where, CONVERSATIONS, type Runner } from './runner';
import type { Summary } from '../src/app/generated/Summary';

/**
 * Taking a message back, against the real runner: the runner puts the words in
 * the conversation's draft, once, and the phone gets them as it gets any draft.
 * A stand-in Mac here would only say what we believe the runner does.
 *
 * The stub CLI keeps a message starting `queued:` waiting, unread, as a message
 * sent mid-turn waits.
 */

const PORT = 18098;
const STATIC = 'frontend/dist/console-build/browser';

let runner: Runner;

test.beforeAll(async () => {
  runner = await start(PORT, STATIC);
});

test.afterAll(() => runner?.stop());

const devices: BrowserContext[] = [];

// Requests held by a route are let go first: one still in flight when its page
// closes fails, and is counted against the test.
test.afterEach(async () => {
  const closing = devices.splice(0, devices.length);
  for (const page of closing.flatMap((context) => context.pages())) {
    await page.unrouteAll({ behavior: 'ignoreErrors' });
  }
  await Promise.all(closing.map((context) => context.close()));
});

/** Pick up a fixture conversation, so it has a process to take messages. */
async function live(talk: string): Promise<string> {
  const res = await fetch(`${runner.base}/api/sessions`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ dir: where(), resume: talk }),
  });
  if (!res.ok) throw new Error(`resume ${talk}: ${res.status} ${await res.text()}`);
  return ((await res.json()) as Summary).id;
}

/** The phone, on a conversation, once its drafts are syncing. */
async function phone(browser: Browser, id: string): Promise<Page> {
  const context = await browser.newContext();
  devices.push(context);
  const page = await context.newPage();
  const syncing = page.waitForRequest((request) => request.url().includes('/api/sync/drafts'));
  await page.goto(`${runner.base}/s/${id}`);
  await syncing;
  return page;
}

const box = (page: Page) => page.locator('textarea[name="text"]');

/** Long enough for the draft poll to have come round again. */
const ANOTHER_POLL_MS = 6_000;

test('a message taken back goes before what is being written, once however often tapped', async ({
  browser,
}) => {
  const id = await live(CONVERSATIONS[0]);
  const page = await phone(browser, id);
  await box(page).fill('queued: rename the branch');
  await page.getByRole('button', { name: 'send', exact: true }).click();
  const mine = page.locator('li.entry', { hasText: 'queued: rename the branch' });
  await expect(mine).toContainText('waiting to be read', { timeout: 15_000 });

  // Typed and tapped at once: the runner must hold "and then" before it puts
  // the words in front, or the two run together.
  await box(page).fill('and then');
  await mine.getByRole('button', { name: 'take back' }).click({ clickCount: 3 });
  await expect(box(page)).toHaveValue('queued: rename the branch\n\nand then', {
    timeout: 15_000,
  });
  await expect(mine).toHaveCount(0);
  await page.waitForTimeout(ANOTHER_POLL_MS);
  await expect(box(page)).toHaveValue('queued: rename the branch\n\nand then');
});

test('a message whose take-back answer was lost still comes back, once', async ({ browser }) => {
  const id = await live(CONVERSATIONS[1]);
  const page = await phone(browser, id);
  // The runner takes every send, and the phone hears none of the answers: it
  // keeps its own copy, as after a receipt lost going offline.
  await page.route('**/api/sessions/*/input', async (route) => {
    await route.fetch();
    await route.abort();
  });
  let lost = false;
  await page.route('**/api/sessions/*/withdraw', async (route) => {
    if (lost) return route.continue();
    lost = true;
    await route.fetch();
    await route.abort();
  });
  await box(page).fill('queued: check the build');
  await page.getByRole('button', { name: 'send', exact: true }).click();
  const mine = page.locator('li.entry', { hasText: 'queued: check the build' });
  await expect(mine).toContainText('sent, no answer yet', { timeout: 15_000 });

  // Taken back on the runner; the answer never arrives.
  await mine.getByRole('button', { name: 'take back' }).click();
  await expect(box(page), 'the words are in the draft regardless').toHaveValue(
    'queued: check the build',
    { timeout: 15_000 },
  );
  // Asked again: answered again, and nothing written twice.
  await mine.getByRole('button', { name: 'take back' }).click();
  await expect(mine).toHaveCount(0, { timeout: 15_000 });
  await page.waitForTimeout(ANOTHER_POLL_MS);
  await expect(box(page)).toHaveValue('queued: check the build');
});
