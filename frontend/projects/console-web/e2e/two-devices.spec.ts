import { test, expect, type Browser, type BrowserContext, type Page } from '@playwright/test';

import { start, CONVERSATIONS, type Runner } from './runner';

/**
 * Two devices, one runner, a real draft crossing between them.
 *
 * This is the only test in the repo where the Rust runner and a real
 * browser meet. `console/tests/suite/drafts.rs` merges documents Rust wrote;
 * `drafts.spec.ts` drives the client against a `vi.fn()` answering what
 * TypeScript expects. Both pass while the two disagree about the wire — which is
 * how four sync defects reached the phone, each found by hand with two browsers
 * and none by a test.
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
  // Ready means the REPLICATION is running, not that the document loaded.
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

// Closed, or the suite loads itself off its own feet. Every device is an
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
 * Without this the tests race their own premise. "type on the Mac, then
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
  // The defect this exists for. The composer records itself on open with
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
  // The control, and it is why the rest are evidence. Each test above
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

test('two devices writing at once keep both, with nobody asked to choose', async ({ browser }) => {
  // The behaviour this whole design exists for. Two half-written thoughts
  // become one text holding both. What was here before drew a card with four
  // buttons and made a person pick; `console/src/drafts.rs` carries the
  // measurement that ended it — 44 refusals, and in 13 of 15 the two sides were
  // the same sentence a few characters apart.
  const talk = CONVERSATIONS[4];
  const mac = await device(browser, `/s/${talk}`);
  await expect(box(mac)).toBeVisible();

  const context = await browser.newContext();
  devices.push(context);
  const phone = await context.newPage();
  await phone.goto(`${runner.base}/s/${talk}`);
  await expect(box(phone)).toBeVisible();

  // The tunnel drops. `abort`, not an empty answer: a reply saying "nothing
  // landed" is a SUCCESS to the client, which would then never retry — and the
  // queued write is the whole mechanism being tested.
  await phone.route('**/api/sync/drafts*', (route) => route.abort());

  // Both write, neither knowing about the other.
  await type(mac, 'from the mac');
  await type(phone, 'from the phone');
  await phone.waitForTimeout(2_000);

  await phone.unroute('**/api/sync/drafts*');
  // `inputValue`, not `toContainText`. A textarea's text CONTENT is what
  // the markup shipped; what a person typed is its value, and asserting on the
  // former reads empty on every controlled box in this app.
  for (const page of [phone, mac]) {
    await expect.poll(() => box(page).inputValue(), { timeout: 30_000 }).toContain('from the mac');
    await expect
      .poll(() => box(page).inputValue(), { timeout: 30_000 })
      .toContain('from the phone');
  }
  await expect(phone.getByText('this was also written elsewhere')).toBeHidden();

  // And the two devices agree on ONE text, character for character. Each
  // holding both halves in a different order would be a merge that converged on
  // nothing, which reads as working until somebody presses send.
  const settled = await box(phone).inputValue();
  await expect(box(mac)).toHaveValue(settled, { timeout: 30_000 });
});

test('a device that is only watching is never asked about words it did not write', async ({
  browser,
}) => {
  // The shape the reported defect took: one person typing, the other device
  // merely open. Under the old protocol the watcher republished what it had just
  // received, against a master the typist had already moved past.
  const talk = CONVERSATIONS[5];
  const complaints: string[] = [];
  const typist = await device(browser, `/s/${talk}`);
  const watcher = await device(browser, `/s/${talk}`);
  for (const page of [typist, watcher]) {
    page.on('console', (message) => {
      if (message.text().includes('draft sync:')) complaints.push(message.text());
    });
  }

  await type(typist, 'one');
  await expect(box(watcher)).toHaveValue('one', { timeout: 20_000 });

  // Straight through, without waiting for any of it to reach the runner.
  for (const said of ['one t', 'one tw', 'one two', 'one two t', 'one two th']) {
    await box(typist).fill(said);
  }
  await runnerHolds(talk, 'one two th');
  await expect(box(watcher)).toHaveValue('one two th', { timeout: 20_000 });
  expect(complaints, `the sync complained:\n${complaints.join('\n')}`).toEqual([]);
});

test('a long message typed straight through arrives whole', async ({ browser }) => {
  // A keystroke must be ONE insert, not a rewrite of the sentence. If the
  // client replaced the whole text on every change, two devices would merge as
  // two people retyping at once and the result would be shredded. Sixty
  // keystrokes with a second device watching is what that looks like if it is
  // wrong.
  const talk = CONVERSATIONS[6];
  const typist = await device(browser, `/s/${talk}`);
  const watcher = await device(browser, `/s/${talk}`);
  await expect(box(watcher)).toBeVisible();

  const whole = 'a sentence typed one character at a time, all the way to the end';
  let said = '';
  for (const letter of whole) {
    said += letter;
    await box(typist).fill(said);
  }

  await runnerHolds(talk, whole);
  await expect(box(watcher)).toHaveValue(whole, { timeout: 20_000 });
});
