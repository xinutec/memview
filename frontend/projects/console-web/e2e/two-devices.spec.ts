import { test, expect, type Browser, type Page } from '@playwright/test';

import { start, CONVERSATIONS, type Runner } from './runner';

/**
 * Two devices, one runner, a real draft crossing between them.
 *
 * ⚠ **This is the only test in the repo where the Rust runner and the RxDB
 * client meet.** `console/tests/suite/drafts.rs` drives the protocol with
 * documents Rust wrote; `drafts.spec.ts` drives the client against a `vi.fn()`
 * answering what TypeScript expects. Both pass while the two disagree about the
 * wire — which is how four sync defects reached the phone, each found by hand
 * with two browsers and none by a test.
 *
 * The composer is the subject, not the API: a draft that syncs but never reaches
 * the box is the defect that opening a conversation CLEARED it, which no
 * assertion on the protocol could have caught.
 */

const PORT = 18099;
const STATIC = 'frontend/dist/console-build/browser';

let runner: Runner;

test.beforeAll(async () => {
  runner = await start(PORT, STATIC);
});

test.afterAll(() => runner?.stop());

/** A device: its own context, so its own IndexedDB and its own replication. */
async function device(browser: Browser, at: string): Promise<Page> {
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(`${runner.base}${at}`);
  return page;
}

const box = (page: Page) => page.locator('textarea[name="text"]');

test('a draft written on one device reaches the other', async ({ browser }) => {
  const [talk] = CONVERSATIONS;
  const mac = await device(browser, `/s/${talk}`);
  await expect(box(mac)).toBeVisible();
  await box(mac).fill('half a thought typed on the Mac');

  // A second device that has never seen this conversation. It must pull the
  // draft, not merely fail to overwrite it.
  const phone = await device(browser, `/s/${talk}`);
  await expect(box(phone)).toHaveValue('half a thought typed on the Mac', { timeout: 20_000 });
});

test('opening a conversation on a second device does not clear its draft', async ({ browser }) => {
  // ⚠ **The defect this exists for.** The composer records itself on open with
  // an empty box; the draft then arrives from the other device; and the empty
  // record — no longer a first write, because a document now exists — was
  // written over it. MERELY LOOKING at a conversation on the phone deleted what
  // had been typed on the Mac. It survives on the phone below AND back on the
  // Mac, because the loss showed up on the device that did the typing.
  const talk = CONVERSATIONS[1];
  const mac = await device(browser, `/s/${talk}`);
  await box(mac).fill('words that must survive being looked at');

  const phone = await device(browser, `/s/${talk}`);
  await expect(box(phone)).toHaveValue('words that must survive being looked at', {
    timeout: 20_000,
  });

  await phone.reload();
  await expect(box(phone)).toHaveValue('words that must survive being looked at', {
    timeout: 20_000,
  });
  await expect(box(mac)).toHaveValue('words that must survive being looked at');
});

test('a draft cleared on one device clears on the other', async ({ browser }) => {
  // A cleared draft is a tombstone, not a removal — otherwise the other device
  // pulls the old text back and the message somebody deliberately dropped
  // reappears. That is the "sent message came back as a draft" report.
  const talk = CONVERSATIONS[2];
  const mac = await device(browser, `/s/${talk}`);
  await box(mac).fill('this one gets thrown away');

  const phone = await device(browser, `/s/${talk}`);
  await expect(box(phone)).toHaveValue('this one gets thrown away', { timeout: 20_000 });

  await box(phone).fill('');
  await expect(box(mac)).toHaveValue('', { timeout: 20_000 });
});

test('the three above mean something: with sync blocked, nothing arrives', async ({ browser }) => {
  // ⚠ **The control, and it is why the rest are evidence.** Each test above
  // passes just as well if the second device were somehow reading the first's
  // storage, or if the box were being filled by something other than the
  // runner. Here the phone is cut off from `/api/sync/drafts` alone — same
  // fixture, same conversation, same typing — and the draft must NOT appear.
  //
  // Blocked on the PHONE only: the Mac still pushes, so this isolates the pull
  // rather than switching sync off everywhere and proving less.
  const talk = CONVERSATIONS[3];
  const mac = await device(browser, `/s/${talk}`);
  await box(mac).fill('this must not cross');

  const context = await browser.newContext();
  const page = await context.newPage();
  await page.route('**/api/sync/drafts*', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ documents: [], checkpoint: { rev: 0 } }),
    }),
  );
  await page.goto(`${runner.base}/s/${talk}`);
  await expect(box(page)).toBeVisible();
  // Long enough that the interval has come round more than once.
  await page.waitForTimeout(12_000);
  await expect(box(page)).toHaveValue('');
});
