import { test, expect, type BrowserContext } from '@playwright/test';

import { start, where, CONVERSATIONS, type Runner } from './runner';
import type { Summary } from '../src/app/generated/Summary';

/**
 * A message sent, and seen through to being read, against the real runner. The
 * stub CLI keeps a message starting `queued:` waiting, unread.
 */

const PORT = 18096;
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

test('a message sent stays on screen when an older answer arrives after it', async ({
  browser,
}) => {
  // On a slow link a poll asked before the send was answered after it, and the
  // phone took its older word: the message vanished until the next poll.
  const context = await browser.newContext();
  devices.push(context);
  const page = await context.newPage();
  await page.route('**/api/state', async (route) => {
    const answer = await route.fetch();
    await new Promise((later) => setTimeout(later, 3_000));
    await route.fulfill({ response: answer });
  });
  const id = await live(CONVERSATIONS[2]);
  await page.goto(`${runner.base}/s/${id}`);
  const box = page.locator('textarea[name="text"]');
  await box.waitFor();
  // A poll leaves; its answer, taken now, comes three seconds later.
  await page.waitForRequest('**/api/state');
  await box.fill('queued: then the tests');
  await page.getByRole('button', { name: 'send', exact: true }).click();
  const mine = page.locator('li.entry', { hasText: 'queued: then the tests' });
  await expect(mine).toContainText('waiting to be read');

  // Through the late answer and past it, it never leaves the screen.
  const gone = page.waitForFunction(
    () =>
      ![...document.querySelectorAll('li.entry')].some((li) =>
        li.textContent?.includes('queued: then the tests'),
      ),
    undefined,
    { timeout: 6_000, polling: 'raf' },
  );
  await expect(gone).rejects.toThrow(/Timeout/);
});

test('a message stays on screen between the session starting on it and its echo', async ({
  browser,
}) => {
  // The CLI says it started on a message, then runs its hooks, then echoes it:
  // 1.8s apart, measured. A poll in between said "read" before the conversation
  // had it, and the message left the screen until the echo.
  const context = await browser.newContext();
  devices.push(context);
  const page = await context.newPage();
  const id = await live(CONVERSATIONS[3]);
  await page.goto(`${runner.base}/s/${id}`);
  const box = page.locator('textarea[name="text"]');
  await box.waitFor();
  await page.waitForTimeout(1_000);
  await box.fill('hooked: rename the branch');
  await page.getByRole('button', { name: 'send', exact: true }).click();
  const mine = page.locator('li.entry', { hasText: 'hooked: rename the branch' });
  await expect(mine).toBeVisible();
  const gone = page.waitForFunction(
    () =>
      ![...document.querySelectorAll('li.entry')].some((li) =>
        li.textContent?.includes('hooked: rename the branch'),
      ),
    undefined,
    { timeout: 5_000, polling: 'raf' },
  );
  // Inside the window: the page asks again, as it does coming back to the front.
  await page.waitForTimeout(500);
  await page.evaluate(() => document.dispatchEvent(new Event('visibilitychange')));
  // Read and not yet echoed it is shown as it will be in the conversation: no
  // "waiting", and nothing to take back.
  await expect(mine.getByRole('button', { name: 'take back' })).toHaveCount(0);
  await expect(mine).not.toContainText('waiting');
  await expect(gone).rejects.toThrow(/Timeout/);
});
