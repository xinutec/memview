import { test, expect, type Browser, type BrowserContext, type Page } from '@playwright/test';

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

const devices: BrowserContext[] = [];

/** A device: its own context, so its own IndexedDB and its own replication. */
async function device(browser: Browser, at: string): Promise<Page> {
  const context = await browser.newContext();
  devices.push(context);
  const page = await context.newPage();
  // ⚠ **Ready means the REPLICATION is running, not that the document loaded.**
  // `goto` resolves on load and the textarea is in the first render, so
  // Playwright's actionability check passes on a box Angular has not finished
  // binding: the `fill` lands in the DOM and no write is ever made. Seen as a
  // device that typed and never pushed, with the runner's log silent for the
  // whole timeout. Armed BEFORE the navigation, or the request is missed.
  const syncing = page.waitForRequest((request) => request.url().includes('/api/sync/drafts'));
  await page.goto(`${runner.base}${at}`);
  await syncing;
  return page;
}

// ⚠ **Closed, or the suite loads itself off its own feet.** Every device is an
// Angular app polling the runner and replicating on an interval; left open they
// accumulate across tests, and the LAST tests start failing on a runner that is
// simply too busy to answer. Seen as a timeout that moved between tests from one
// run to the next, which is what says it is contention and not a defect.
test.afterEach(async () => {
  await Promise.all(devices.splice(0, devices.length).map((context) => context.close()));
});

const box = (page: Page) => page.locator('textarea[name="text"]');

/** Type, and confirm the component took it before anything waits on the wire.
 *  Separates "Angular never saw the keystroke" from "the push never happened". */
async function type(page: Page, text: string): Promise<void> {
  await box(page).fill(text);
  await expect(box(page)).toHaveValue(text);
}

/**
 * Wait until the RUNNER holds `text` for `talk`.
 *
 * ⚠ **Without this the tests race their own premise.** "type on the Mac, then
 * open the phone" assumes the Mac has PUSHED, which nothing guarantees — and
 * when it has not, the phone opens, pulls nothing, and waits out an interval it
 * was never given time for. That failed under gate load while passing alone.
 * Asserting here also splits the two failures apart: this one says the Mac never
 * pushed, the assertion after it says the phone never pulled.
 */
async function runnerHolds(talk: string, text: string): Promise<void> {
  await expect
    .poll(
      async () => {
        const body: unknown = await fetch(`${runner.base}/api/sync/drafts?since=0`).then((res) =>
          res.json(),
        );
        if (typeof body !== 'object' || body === null || !('documents' in body)) return undefined;
        const { documents } = body;
        if (!Array.isArray(documents)) return undefined;
        const rows: unknown[] = documents;
        for (const row of rows) {
          if (typeof row !== 'object' || row === null) continue;
          if (!('ulid' in row) || row.ulid !== talk) continue;
          if (!('text' in row) || typeof row.text !== 'string') continue;
          return row.text;
        }
        return undefined;
      },
      {
        timeout: 30_000,
        message:
          `the runner never took "${text}" for ${talk}.\n` +
          `Either the client never sent it, or the runner refused — its log says:\n` +
          runner.said().slice(-2000),
      },
    )
    .toBe(text);
}

test('a draft written on one device reaches the other', async ({ browser }) => {
  const [talk] = CONVERSATIONS;
  const mac = await device(browser, `/s/${talk}`);
  await expect(box(mac)).toBeVisible();
  await type(mac, 'half a thought typed on the Mac');
  await runnerHolds(talk, 'half a thought typed on the Mac');

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
  await type(mac, 'words that must survive being looked at');
  await runnerHolds(talk, 'words that must survive being looked at');

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
  await type(mac, 'this one gets thrown away');
  await runnerHolds(talk, 'this one gets thrown away');

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
  await type(mac, 'this must not cross');
  await runnerHolds(talk, 'this must not cross');

  const context = await browser.newContext();
  devices.push(context);
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

test('two devices writing at once are offered both, and the combination crosses back', async ({
  browser,
}) => {
  // ⚠ **The feature in this task's title, and nothing has ever driven it.** The
  // clash UI and its four buttons are drawn by `session-view.html`; `mine first`
  // and `theirs first` are the reason the screen exists rather than a silent
  // last-writer-wins. Two half-written thoughts join into one message, which no
  // automatic rule could do for somebody.
  const talk = CONVERSATIONS[4];
  const mac = await device(browser, `/s/${talk}`);
  await expect(box(mac)).toBeVisible();

  const context = await browser.newContext();
  devices.push(context);
  const phone = await context.newPage();
  await phone.goto(`${runner.base}/s/${talk}`);
  await expect(box(phone)).toBeVisible();

  // The tunnel drops. `abort`, not an empty answer: a reply saying "nothing
  // landed" is a SUCCESS to the replication, which would then never retry — and
  // the queued write is the whole mechanism being tested.
  await phone.route('**/api/sync/drafts*', (route) => route.abort());

  // Both write, neither knowing about the other.
  await type(mac, 'from the mac');
  await type(phone, 'from the phone');
  await phone.waitForTimeout(2_000);

  await phone.unroute('**/api/sync/drafts*');

  // ⚠ **Both texts IN FULL, not a diff and not a count.** The choice is between
  // two things somebody wrote, and it cannot be made from a summary.
  const clash = phone.locator('.clash');
  await expect(clash).toBeVisible({ timeout: 30_000 });
  await expect(clash).toContainText('from the mac');
  await expect(clash).toContainText('from the phone');

  // `theirs first`: the other device's words, then this one's.
  await phone.getByRole('button', { name: 'theirs first' }).click();
  await expect(box(phone)).toHaveValue('from the mac\n\nfrom the phone');

  // And the combination is a draft like any other, so it crosses back.
  await expect(box(mac)).toHaveValue('from the mac\n\nfrom the phone', { timeout: 30_000 });
});

test('the other order is the other order, and it crosses back too', async ({ browser }) => {
  // ⚠ **`mine first` was drawn and driven by nothing.** The branch above covers
  // `theirs first`, and the two differ only in which side of the join comes
  // first — which is exactly why an untested one is cheap to get backwards and
  // impossible to notice: both produce a plausible message, and only the person
  // who wrote the two halves knows which order they meant.
  const talk = CONVERSATIONS[5];
  const mac = await device(browser, `/s/${talk}`);
  await expect(box(mac)).toBeVisible();

  const context = await browser.newContext();
  devices.push(context);
  const phone = await context.newPage();
  await phone.goto(`${runner.base}/s/${talk}`);
  await expect(box(phone)).toBeVisible();

  await phone.route('**/api/sync/drafts*', (route) => route.abort());

  await type(mac, 'from the mac');
  await type(phone, 'from the phone');
  await phone.waitForTimeout(2_000);

  await phone.unroute('**/api/sync/drafts*');

  const clash = phone.locator('.clash');
  await expect(clash).toBeVisible({ timeout: 30_000 });

  // `mine first`: THIS device's words, then the other's — the mirror of the
  // assertion above, on the same two halves, so a join written the wrong way
  // round cannot satisfy both.
  await phone.getByRole('button', { name: 'mine first' }).click();
  await expect(box(phone)).toHaveValue('from the phone\n\nfrom the mac');

  await expect(box(mac)).toHaveValue('from the phone\n\nfrom the mac', { timeout: 30_000 });
});
