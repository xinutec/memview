import {
  BUSY_BAR,
  CROWDED,
  ENDED,
  MIXED,
  NEXT,
  ON_DISK,
  RUNNING,
  STATE,
  expect,
  expectCleanLayout,
  expectIconFontLoaded,
  expectNoClippedText,
  expectNothingPaintsOver,
  expectOneLine,
  expectThumbTargets,
  expectViewportIsPhone,
  first,
  mockRunner,
  test,
  type Conversation,
  type Locator,
  type LoginCode,
  type LoginResult,
  type LoginStarted,
  type Overview,
} from './console';

test.use({ serviceWorkers: 'block' });

test('the suite really runs at phone geometry', async ({ page }) => {
  await mockRunner(page);
  await page.goto('/');
  await expectViewportIsPhone(page);
});

test('session list — deep paths and long instructions @ phone width', async ({
  page,
}, testInfo) => {
  await mockRunner(page);
  await page.goto('/');
  await page.getByText('decode').first().waitFor();
  await expectIconFontLoaded(page);
  await expectCleanLayout(page, testInfo);
  await expectThumbTargets(page);
});

test('starting a session is behind one button, not in the way @ phone width', async ({
  page,
}, testInfo) => {
  // The list is what this page is opened for — a dozen conversations, scanned
  // for the one that is waiting, so starting one is behind a button.
  await mockRunner(page);
  let sent: Record<string, unknown> | undefined;
  await page.route('**/api/sessions', (r) => {
    sent = r.request().postDataJSON() as Record<string, unknown>;
    return r.fulfill({ json: RUNNING });
  });
  await page.goto('/');
  await page.getByText('decode').first().waitFor();

  // Nothing but conversations on the page itself.
  await expect(page.getByLabel('where'), 'the form is still in the reading').toHaveCount(0);
  const add = page.getByRole('button', { name: 'start a new session' });
  await expect(add).toBeVisible();

  await add.click();
  const where = page.getByLabel('where');
  await expect(where, 'the form did not open').toBeVisible();
  // Prefilled with where sessions ACTUALLY run, not with the first
  // repository alphabetically. The old default was a real directory nothing had
  // ever been started in, which is the kind of wrong that looks deliberate — see
  // [[SessionsView.commonest]]. Here that is the busiest session's own
  // directory, which both mocked sessions share.
  await expect(where).toHaveValue(RUNNING.dir);
  // Opening the sheet must not open a list. The field opens on `~/Code`,
  // a prefix of every repository, and the native `<datalist>` this replaced
  // matched the whole value — so pressing + painted all 24 over the phone.
  const offered = page.locator('.suggestions button');
  await expect(offered, 'the default offers nothing').toHaveCount(0);
  await where.fill('/home/example/Code/mem');
  await expect(offered).toHaveText(['memview']);
  // And nothing once the answer is typed, so no row is left over the button.
  await where.fill('/home/example/Code/memview');
  await expect(offered).toHaveCount(0);
  // Scoped to the sheet: it is the only thing on screen that matters now, and
  // the list behind it is still in the DOM.
  await expectCleanLayout(page, testInfo, { root: 'mat-bottom-sheet-container' });

  // The opening-instruction field is gone: it duplicated the composer this sheet
  // navigates straight to, and did exactly the same thing.
  await expect(page.getByLabel('what to do (optional)')).toHaveCount(0);

  await where.fill('/home/example/Code/memview');
  await page.getByRole('button', { name: /start a session/ }).click();

  await expect.poll(() => sent).toMatchObject({ dir: '/home/example/Code/memview', prompt: '' });
});

test('starting a session leaves no history entry for the sheet behind @ phone width', async ({
  page,
}) => {
  // The sheet closes and the app navigates at once, but the sheet's history
  // entry is only taken back after its exit animation — by then the session's
  // entry is on top, so the sheet's is stranded under it and back from the
  // session spends a press on nothing.
  await mockRunner(page);
  await page.route('**/api/sessions', (r) => r.fulfill({ json: RUNNING }));
  await page.goto('/');
  await page.getByText('decode').first().waitFor();
  await page.getByRole('button', { name: 'start a new session' }).click();
  await page.getByLabel('where').fill('/home/example/Code/memview');
  await page.getByRole('button', { name: /start a session/ }).click();
  await expect(page).toHaveURL(new RegExp(`/s/${RUNNING.id}`));
  await page.locator('mat-bottom-sheet-container').waitFor({ state: 'detached' });

  await page.goBack();
  await expect(page).toHaveURL(/\/$/);
  // Polled: back lands on the leftover entry first, and the step past it follows
  // the popstate. Without that step the state stays the sheet's.
  await expect
    .poll(
      () =>
        page.evaluate(
          () =>
            typeof history.state === 'object' &&
            history.state !== null &&
            'overlay' in history.state,
        ),
      { message: "back landed on the sheet's leftover entry" },
    )
    .toBe(false);
});

test('session list — a dozen sessions do not reach the build stamp @ phone width', async ({
  page,
}, testInfo) => {
  await mockRunner(page);
  await page.route('**/api/state', (r) => r.fulfill({ json: CROWDED }));
  await page.goto('/');
  await expect(page.locator('.page .session')).toHaveCount(CROWDED.sessions.length);
  await expectNothingPaintsOver(page, '.build');
  await expectCleanLayout(page, testInfo);

  // And the last card has to clear the button, which only shows at the
  // bottom. `.add` is `fixed`, so a list with nothing under it scrolls its last
  // card straight underneath — measured at 56×56, with the button's own touch
  // target answering `elementFromPoint` there, so the tap went to it as well.
  // The same shape as the composer covering the last entry's buttons, which
  // `app.scss` records; the cure is the same, room at the foot of the list.
  await page.evaluate(() => {
    const list = document.querySelector('.page');
    if (list) list.scrollTop = list.scrollHeight;
  });
  await expectNothingPaintsOver(page, '.add');
});

test('session list — awake first, and what is off says so @ phone width', async ({ page }) => {
  // What the page is opened to answer: which of these is working. Everything
  // that exists is on it — a conversation on disk is a row like any other, not a
  // count behind a disclosure — and being off is a property of the row.
  await mockRunner(page);
  await page.route('**/api/state', (r) => r.fulfill({ json: MIXED }));
  await page.route('**/api/past', (r) => r.fulfill({ json: ON_DISK }));
  await page.goto('/');
  await expect(page.locator('.session')).toHaveCount(6);

  expect(
    await page.locator('.session .place').allInnerTexts(),
    'the order is working, blocked, idle, then everything that is off',
  ).toEqual(['working', 'blocked', 'idle-one', 'finished', 'older', 'held-elsewhere']);

  // Off is visible as off, without reading a word: the three that are not
  // running are dimmed and the three that are are not.
  const dimmed = await page.evaluate(() =>
    [...document.querySelectorAll('.session')].map(
      (row) => Number.parseFloat(getComputedStyle(row).opacity) < 1,
    ),
  );
  expect(dimmed, 'the ones that are not on have to look it').toEqual([
    false,
    false,
    false,
    true,
    true,
    true,
  ]);

  // And the one another process is holding cannot be picked up by tapping it.
  const held = page.locator('.session', { hasText: 'held-elsewhere' });
  await expect(held).toHaveClass(/disabled/);
  await expect(page.locator('.caution')).toBeVisible();
});

test('session list — the time is when it last did something @ phone width', async ({ page }) => {
  // The two dates a session has, and the card must show the second.
  // `started` is when the console picked the process up — carried across an
  // in-place upgrade, reset by a restart. `touched` is when the transcript was
  // last written, which is when the conversation last moved. The console's own
  // session showed `13h ago` on a card while it was mid-answer, because the
  // card was reading the first one.
  const DAY = 24 * 60 * 60;
  await mockRunner(page);
  await page.route('**/api/state', (r) =>
    r.fulfill({
      json: {
        ...STATE,
        sessions: [
          {
            ...RUNNING,
            name: 'worked-all-night',
            // Picked up a day ago, answering this minute.
            started: Math.floor(Date.now() / 1000) - DAY,
            touched: Date.now() - 30_000,
          },
        ],
      } satisfies Overview,
    }),
  );
  await page.goto('/');
  await expect(page.locator('.session')).toHaveCount(1);
  const facts = await page.locator('.session .facts').first().innerText();
  expect(facts, 'the card is dating the process instead of the conversation').toContain('just now');
  expect(facts).not.toContain('24h ago');
});

test('session list — how full each conversation is @ phone width', async ({ page }, testInfo) => {
  // The question the list could not answer: which of these is about to compact.
  // A running session divides by the window it declared; a conversation on disk
  // has no window to divide by — the CLI declares it on the result line, which
  // never reaches the transcript — so it names the unit instead of implying it.
  await mockRunner(page);
  await page.route('**/api/state', (r) =>
    r.fulfill({
      json: {
        ...STATE,
        sessions: [{ ...RUNNING, name: 'running', context: 496_231, window: 1_000_000 }],
      } satisfies Overview,
    }),
  );
  await page.route('**/api/past', (r) =>
    r.fulfill({
      json: [{ ...first(ON_DISK), name: 'on-disk', context: 340_000 }] satisfies Conversation[],
    }),
  );
  await page.goto('/');
  await expect(page.locator('.session')).toHaveCount(2);

  const facts = await page.locator('.session .facts').allInnerTexts();
  expect(facts[0], 'a running session knows what it is full of').toContain('496k / 1M');
  expect(facts[1], 'a bare count with no denominator has to name its unit').toContain(
    '340k tokens',
  );
  // And the history is not here. It was, and a fourth fact wrapped the row
  // onto a second line — so the size moved to the details sheet, which is where
  // a fact you look up belongs rather than one you scan a list by.
  expect(facts.join(' '), 'the size is back on the card').not.toContain('MB');

  // The same fullness as a bar on the card's bottom edge, only where there is a
  // window to divide by.
  const fill = page.locator('.session').nth(0).locator('mat-progress-bar.fill');
  await expect(fill).toHaveAttribute('aria-valuenow', '50');
  const heat = (bar: Locator) =>
    bar.evaluate((el: HTMLElement) =>
      ['--warm', '--hot'].map((name) => el.style.getPropertyValue(name)).join(' '),
    );
  expect(await heat(fill)).toBe('0 0');
  // Below the facts, not over them: the card keeps its bottom padding.
  const factsBox = await page.locator('.session').nth(0).locator('.facts').boundingBox();
  const fillBox = await fill.boundingBox();
  expect(factsBox!.y + factsBox!.height).toBeLessThanOrEqual(fillBox!.y);
  await expect(page.locator('.session').nth(1).locator('mat-progress-bar.fill')).toHaveCount(0);

  // Towards amber from 80%, squared; towards red from 90%, linear; all red from 95%.
  for (const [context, want] of [
    [850_000, '0.25 0'],
    [930_000, '1 0.6'],
    [960_000, '1 1'],
  ] as const) {
    await page.route('**/api/state', (r) =>
      r.fulfill({
        json: {
          ...STATE,
          sessions: [{ ...RUNNING, name: 'running', context, window: 1_000_000 }],
        } satisfies Overview,
      }),
    );
    await page.reload();
    expect(await heat(page.locator('.session').nth(0).locator('mat-progress-bar.fill'))).toBe(want);
  }

  // The row wraps rather than clipping, and what it must not do is push the card
  // sideways or land on top of itself.
  await expectCleanLayout(page, testInfo);
});

test('session list — the facts sit on one centre line, as on the session page @ phone width', async ({
  page,
}) => {
  // The icons are taller than the text beside them; stretched to the row, each
  // text sat at the top of its own box. 0.8px here, plain to see at a phone's
  // font scale.
  await mockRunner(page);
  await page.route('**/api/state', (r) =>
    r.fulfill({
      json: {
        ...STATE,
        sessions: [{ ...RUNNING, context: 122_000, window: 1_000_000 }],
      } satisfies Overview,
    }),
  );
  await page.goto('/');
  const off = await page
    .locator('.session .facts')
    .first()
    .evaluate((row) => {
      const box = row.getBoundingClientRect();
      const middle = box.y + box.height / 2;
      return [...row.children].flatMap((item) => {
        // The ink, not the box: a stretched box is centred whatever it holds. An
        // icon is its box; its text is a glyph name in a line box of its own.
        const icon = item.matches('mat-icon') ? item : item.querySelector('mat-icon');
        const text = item.matches('mat-icon')
          ? undefined
          : [...item.childNodes].find(
              (n) => n.nodeType === Node.TEXT_NODE && n.textContent?.trim(),
            );
        const words = document.createRange();
        if (text) words.selectNodeContents(text);
        return [icon?.getBoundingClientRect(), text && words.getBoundingClientRect()]
          .filter((rect) => rect !== undefined)
          .map((rect) => Math.round((rect.y + rect.height / 2 - middle) * 10) / 10)
          .filter((dy) => Math.abs(dy) > 0.5)
          .map((dy) => `${item.className || item.tagName}: ${dy}px`);
      });
    });
  expect(off).toEqual([]);
});

test('session list — a session waiting for you is yellow, not the green on every card @ phone width', async ({
  page,
}) => {
  // Which session needs you is what the list is for. Material's yellow 90 on 10.
  await mockRunner(page);
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.goto('/');
    const pill = page.locator('.waiting').first();
    await expect(pill, scheme).toHaveCSS('background-color', 'rgb(234, 234, 0)');
    await expect(pill, scheme).toHaveCSS('color', 'rgb(29, 29, 0)');
  }
});

test('session list — work still running says so, silence otherwise @ phone width', async ({
  page,
}, testInfo) => {
  // The one thing a card can be doing while it reads as idle. A
  // backgrounded call outlives the turn that started it, so a session sitting at
  // `idle` may still have a build going. Counted by the runner, because the page
  // that knew this before was the session's own — the screen you had to already
  // be on.
  await mockRunner(page);
  await page.route('**/api/state', (r) =>
    r.fulfill({
      json: {
        ...STATE,
        sessions: [
          {
            ...RUNNING,
            id: 'aaaa0000-0000-4000-8000-000000000001',
            name: 'a-session-with-a-name-long-enough-to-crowd',
            busy: undefined,
            waiting: 0,
            background: 1,
          },
          {
            ...RUNNING,
            id: 'aaaa0000-0000-4000-8000-000000000002',
            name: 'quiet',
            busy: undefined,
            waiting: 0,
          },
        ],
      } satisfies Overview,
    }),
  );
  await page.route('**/api/past', (r) => r.fulfill({ json: [] satisfies Conversation[] }));
  await page.goto('/');
  await expect(page.locator('.session')).toHaveCount(2);

  // In the head, beside the status word it qualifies — not down in the
  // facts row with the dates and counts, where it read as one more piece of
  // history rather than as the thing still happening.
  // An icon and the number, as the other counters: the words did not fit.
  const tasks = page
    .locator('.session', { hasText: 'long-enough-to-crowd' })
    .locator('.head .tasks');
  await expect(tasks).toHaveAttribute('title', '1 background task');
  await expect(tasks.locator('mat-icon')).toHaveAttribute('aria-label', '1 background task');
  await expect(tasks).toHaveText(/^\s*\S+\s*1\s*$/);
  await expect(
    page.locator('.session', { hasText: 'long-enough-to-crowd' }).locator('.head'),
  ).toContainText('idle');
  // Nothing at all for a session with none — `0 background tasks` would be a
  // claim, and the harness cannot see a command backgrounded inside a shell.
  await expect(page.locator('.session', { hasText: 'quiet' })).not.toContainText('background');
  await expect(page.locator('.tasks')).toHaveCount(1);

  // The other crowded head: a name long enough to push everything else along,
  // and two things qualifying the status word. See [[expectOneLine]].
  await expectOneLine(page, '.session .head');
  await expectCleanLayout(page, testInfo);
});

test('session list — what each conversation still owes @ phone width', async ({
  page,
}, testInfo) => {
  // The list a session keeps for itself, on the page that lists sessions.
  // These numbers were behind the ⋮ of the session already on screen, which is
  // the one conversation whose state you can already see. The question worth
  // asking is the other one — which of a dozen has work left — and it needs the
  // count for every row, so the runner sweeps them all off disk.
  await mockRunner(page);
  await page.route('**/api/state', (r) =>
    r.fulfill({
      json: {
        ...STATE,
        sessions: [
          { ...RUNNING, id: 'aaaa0000-0000-4000-8000-000000000001', name: 'owing' },
          { ...RUNNING, id: 'aaaa0000-0000-4000-8000-000000000002', name: 'no-list' },
          { ...RUNNING, id: 'aaaa0000-0000-4000-8000-000000000003', name: 'all-done' },
          { ...RUNNING, id: 'aaaa0000-0000-4000-8000-000000000004', name: 'astray' },
        ],
        tasks: {
          sessions: {
            'aaaa0000-0000-4000-8000-000000000001': { open: 3, total: 17, stray: 0 },
            'aaaa0000-0000-4000-8000-000000000003': { open: 0, total: 4, stray: 0 },
            // A conversation nothing is running, which still has the list it kept
            // — and which migrated without deleting what it left behind.
            'bbbb0000-0000-4000-8000-000000000001': { open: 2, total: 9, stray: 5 },
            // Nothing in the service and eleven in the store it replaced.
            // The case the sweep of holders cannot produce on its own: work
            // being filed where nothing reads it. No fraction, just the fault.
            'aaaa0000-0000-4000-8000-000000000004': { open: 0, total: 0, stray: 11 },
          },
          // The holders that are on no card, because they are not conversations.
          elsewhere: [
            { name: 'User', open: 1, total: 12 },
            { name: 'nobody', open: 23, total: 26 },
          ],
        },
        // Two bytes under the cut, as the real one once stood.
        index: { bytes: 24398, ceiling: 24400 },
      } satisfies Overview,
    }),
  );
  await page.route('**/api/past', (r) => r.fulfill({ json: ON_DISK }));
  await page.goto('/');
  await expect(page.locator('.session')).toHaveCount(6);

  await expect(page.locator('.session', { hasText: 'owing' }).locator('.list')).toContainText(
    '3/17',
  );
  // A conversation that is not running says it too. The list is on disk
  // beside the transcript and outlives the process, so a session finished
  // yesterday can still be the one holding three unfinished things — and that is
  // exactly the row somebody scanning this page is looking for.
  await expect(page.locator('.session', { hasText: 'older' }).locator('.list')).toContainText(
    '2/9',
  );
  // A list with nothing left says so rather than disappearing: it is a finished
  // list, which is a different thing from never having kept one.
  await expect(page.locator('.session', { hasText: 'all-done' }).locator('.list')).toContainText(
    '0/4',
  );
  // And nothing at all where there is no list — `0/0` would claim one that had
  // been emptied, and most conversations never open one.
  await expect(page.locator('.session', { hasText: 'no-list' }).locator('.list')).toHaveCount(0);

  // What a conversation left in the store the service replaced. Every file
  // there is re-sent to it 1.75 times per message with its whole body, so this
  // is a fault to clear rather than a quantity to know — in parentheses, beside
  // the fraction, and not part of it.
  // Read as two elements, because the space between them is a margin and not
  // a character: Angular drops whitespace-only nodes, so the text is `2/9(5)`.
  const older = page.locator('.session', { hasText: 'older' });
  await expect(older.locator('.fraction')).toHaveText('2/9');
  await expect(older.locator('.stray')).toHaveText('(5)');
  // And with no fraction at all when the service has never heard of it —
  // work being filed where nothing reads it, which is the case a sweep of the
  // service's own holders cannot produce.
  const astray = page.locator('.session', { hasText: 'astray' });
  await expect(astray.locator('.stray')).toHaveText('(11)');
  await expect(astray.locator('.fraction')).toHaveCount(0);
  // Nothing at all on a session that has cleaned up: the number only ever
  // appears when there is something to do about it.
  await expect(page.locator('.session', { hasText: 'owing' }).locator('.stray')).toHaveCount(0);

  // What is on no card, because it belongs to no conversation. The pile is
  // the one queue nobody is working, and every other thing on this page is drawn
  // per session — so without this line it is invisible here by construction.
  await expect(page.locator('.elsewhere')).toContainText('User 1/12');
  await expect(page.locator('.elsewhere')).toContainText('nobody 23/26');
  // The memory index beside them: its size against where a session's copy is
  // cut. Just under it is fine and plain.
  const index = page.locator('.elsewhere .index');
  await expect(index).toHaveText('MEMORY.md 24,398/24,400');
  await expect(index).not.toHaveClass(/over/);

  // The assertion this feature earned, twice. The chip shipped as an
  // inline-flex box, which reports its icon's baseline rather than its digits',
  // so the count rode 3.8px above everything beside it; and the row it joined
  // was aligned on baselines, which put the status pill 2px below the name. Two
  // different faults, both invisible to every other check here, both found by
  // eye on the phone — see [[expectOneLine]].
  await expectOneLine(page, '.session .head');
  await expectCleanLayout(page, testInfo);
});

test('session list — a memory index past its cut is an error @ phone width', async ({
  page,
}, testInfo) => {
  // Past 24,400 bytes Claude Code hands each session a prefix of MEMORY.md and
  // says nothing about the rest, so the size is the one thing to see first.
  await mockRunner(page);
  await page.route('**/api/state', (r) =>
    r.fulfill({ json: { ...STATE, index: { bytes: 24520, ceiling: 24400 } } satisfies Overview }),
  );
  await page.route('**/api/past', (r) => r.fulfill({ json: ON_DISK }));
  await page.goto('/');
  const index = page.locator('.elsewhere .index');
  await expect(index).toHaveText('MEMORY.md 24,520/24,400');
  await expect(index).toHaveClass(/over/);
  await expectCleanLayout(page, testInfo);
});

test('session list — what each conversation is about, marked as a guess @ phone width', async ({
  page,
}, testInfo) => {
  // The only inference on the page. Everything else on a card is read off
  // a file or a process; this sentence is Haiku's reading of the transcript, so
  // it carries a mark saying who wrote it and when. A guess about a conversation
  // nobody has opened is precisely the one somebody would act on unchecked.
  const written = Date.now() - 9 * 60_000;
  await mockRunner(page);
  await page.route('**/api/state', (r) =>
    r.fulfill({
      json: {
        ...STATE,
        sessions: [{ ...RUNNING, id: 'aaaa0000-0000-4000-8000-000000000001', name: 'health' }],
        gists: {
          'aaaa0000-0000-4000-8000-000000000001': {
            text: 'porting the last matcher gate to Lean and checking it against the golden set',
            at: written,
            name: null,
            bytes: 40_000_000,
          },
          // One for a conversation that is not running, which is where it helps
          // most: a name nobody has opened in a week is a word.
          'bbbb0000-0000-4000-8000-000000000001': {
            text: 'reworking the scanner band probe after the axis turned out to be wrong',
            at: written,
            name: null,
            bytes: 40_000_000,
          },
        },
      } satisfies Overview,
    }),
  );
  await page.route('**/api/past', (r) =>
    r.fulfill({ json: [first(ON_DISK)] satisfies Conversation[] }),
  );
  await page.goto('/');
  await expect(page.locator('.session')).toHaveCount(2);

  await expect(page.locator('.session .gist')).toHaveCount(2);
  await expect(page.locator('.session .gist').first()).toContainText(
    'porting the last matcher gate',
  );
  // Who wrote it and when, on the mark rather than in the sentence — the words
  // are the answer and the provenance is not part of it.
  await expect(page.locator('.session .gist .wrote').first()).toHaveAttribute(
    'aria-label',
    /summary written by a model, 9m ago/,
  );

  await expectCleanLayout(page, testInfo);
});

test('session list — the opening instruction stands in for a missing name @ phone width', async ({
  page,
}) => {
  // It is a fallback, not a fact about the session. What the console keeps
  // is the first prompt it heard, for ever — and a conversation's job drifts, so
  // on anything long-running it describes work finished days ago. On a resumed
  // one it is not even that: the view starts at the seed, so what it kept was
  // the first prompt in the last page of the transcript. Once a session has a
  // name, the name is both current and its own summary.
  await mockRunner(page);
  await page.route('**/api/state', (r) =>
    r.fulfill({
      json: {
        ...STATE,
        sessions: [
          {
            ...RUNNING,
            id: 'aaaa0000-0000-4000-8000-000000000001',
            asked: 'fix the gate',
          },
          {
            ...RUNNING,
            id: 'aaaa0000-0000-4000-8000-000000000002',
            name: 'health',
            asked: 'Proceed',
          },
        ],
      } satisfies Overview,
    }),
  );
  await page.route('**/api/past', (r) => r.fulfill({ json: [] satisfies Conversation[] }));
  await page.goto('/');
  await expect(page.locator('.session')).toHaveCount(2);
  await expect(page.locator('.session .asked')).toHaveCount(1);
  await expect(page.locator('.session .asked')).toHaveText('fix the gate');
  await expect(
    page.locator('.session', { hasText: 'health' }),
    'a named session is still carrying whatever it was last told',
  ).not.toContainText('Proceed');
});

test('the list says working, and how many messages are still queued @ phone width', async ({
  page,
}, testInfo) => {
  // A message sent to a session the page calls idle should land at once, so a
  // wrong `idle` or an invisible pending message reads as a failed send, and the
  // same sentence goes twice.
  await mockRunner(page);
  const busy = { ...RUNNING, busy: undefined, waiting: 0, working: true, unread: 2 };
  await page.route('**/api/state', (r) =>
    r.fulfill({ json: { ...STATE, sessions: [busy, ENDED] } satisfies Overview }),
  );
  await page.goto('/');
  await page.getByText('working').first().waitFor();
  await expect(page.locator('.unread')).toHaveText('2 unread');
  await expectCleanLayout(page, testInfo);
});

test('session list — a blocked session says so first @ phone width', async ({ page }, testInfo) => {
  await mockRunner(page);
  await page.goto('/');
  await page.getByText('waiting for you').waitFor();
  await expectCleanLayout(page, testInfo);
});

test('a refused sign-in says so on the list and on the session @ phone width', async ({
  page,
}, testInfo) => {
  // One login runs every session, so when it lapses they all stop at once and
  // look idle. The cure is on the Mac, and the page says where.
  await mockRunner(page);
  await page.route('**/api/state', (r) =>
    r.fulfill({
      json: {
        ...STATE,
        sessions: [{ ...RUNNING, waiting: 0, signed_out: NEXT }],
      } satisfies Overview,
    }),
  );
  await page.goto('/');
  await expect(page.getByText('signed out', { exact: true })).toHaveCount(1);
  await expect(page.getByText('waiting for you')).toHaveCount(0);
  await expectCleanLayout(page, testInfo);

  await page.goto(`/s/${RUNNING.id}`);
  const banner = page.getByRole('alert').filter({ hasText: 'Signed out.' });
  await expect(banner).toHaveCount(1);
  await expectCleanLayout(page, testInfo, { allow: BUSY_BAR });

  // Signed in from here: a link to open, then the code its page showed.
  let sent = '';
  await page.route('**/api/login', (r) =>
    r.fulfill({
      json: { url: 'https://claude.com/cai/oauth/authorize?state=e2e' } satisfies LoginStarted,
    }),
  );
  await page.route('**/api/login/code', (r) => {
    sent = (r.request().postDataJSON() as LoginCode).code;
    return r.fulfill({ json: { ok: true, said: 'Login successful.' } satisfies LoginResult });
  });
  await banner.getByRole('button', { name: 'Sign in' }).click();
  await expect(page.getByRole('link', { name: 'the sign-in page' })).toHaveAttribute(
    'href',
    'https://claude.com/cai/oauth/authorize?state=e2e',
  );
  await page.getByLabel('code').fill('the-code');
  await expectCleanLayout(page, testInfo, { root: 'mat-bottom-sheet-container' });
  await page.getByRole('button', { name: 'sign in', exact: true }).click();
  await expect(page.getByText('Login successful.')).toHaveCount(1);
  expect(sent).toBe('the-code');
});

test('the login can be renewed from the menu before it lapses @ phone width', async ({ page }) => {
  // Signing in again resets the renewal token's thirty days, so a login that is
  // only getting old never has to lapse.
  let asked = 0;
  await mockRunner(page);
  await page.route('**/api/login', (r) => {
    asked += 1;
    return r.fulfill({
      json: { url: 'https://claude.com/cai/oauth/authorize?state=menu' } satisfies LoginStarted,
    });
  });
  await page.goto(`/s/${RUNNING.id}`);
  await page
    .locator('ui-scaffold')
    .getByRole('button', { name: /what to do with/ })
    .click();
  await page.getByRole('menuitem', { name: 'Sign in again' }).click();
  await expect(page.getByRole('link', { name: 'the sign-in page' })).toHaveCount(1);
  expect(asked).toBe(1);
});

test('what the account has spent is above the list @ phone width', async ({ page }, testInfo) => {
  // Four things on one 412px row — a label, a percentage, a bar and a countdown
  // — and only the bar can give way. The rest are short and fixed, so the
  // failure mode is the bar squeezing to nothing or the row wrapping under it.
  await mockRunner(page);
  await page.route('**/api/state', (r) =>
    r.fulfill({
      json: {
        ...STATE,
        usage: STATE.usage && {
          ...STATE.usage,
          five_hour: { pct: 92, resets_in_ms: 3_600_000 },
        },
      } satisfies Overview,
    }),
  );
  await page.goto('/');
  const strip = page.locator('.usage');
  await strip.waitFor();

  // Above the first session, which is what "at the top" means when both are on
  // the same scrolling page.
  const [top, first] = await Promise.all([
    strip.boundingBox(),
    page.locator('.session').first().boundingBox(),
  ]);
  expect(top!.y + top!.height, 'the strip is not above the first session').toBeLessThanOrEqual(
    first!.y,
  );

  await expect(strip).toContainText('92%');
  await expect(strip).toContainText('66%');
  // The reading's age, on screen rather than in a tooltip a phone cannot reach.
  await expect(strip).toContainText('4h ago');

  // The bar still has room to mean something at this width.
  const bar = await page.locator('.usage .level').first().boundingBox();
  expect(bar!.width, 'the bar was squeezed out by the text around it').toBeGreaterThan(80);

  // A label that wraps is neither clipped nor overflowing, so every
  // assertion above passes while "5 hours" sits on two lines. Measured on the
  // text itself rather than its box: a `Range` over a text node reports one
  // client rect per line it occupies, which is the only direct evidence of a
  // wrap there is.
  const lines = await page.evaluate(() =>
    [...document.querySelectorAll('.usage .label')].map((label) => {
      const range = document.createRange();
      range.selectNodeContents(label);
      return { text: label.textContent ?? '', lines: range.getClientRects().length };
    }),
  );
  expect(lines.length, 'no labels to measure').toBeGreaterThan(0);
  for (const label of lines) {
    expect(label.lines, `"${label.text}" is split over ${label.lines} lines`).toBe(1);
  }

  await expectCleanLayout(page, testInfo);
  await expectNoClippedText(page, testInfo, '.usage');
});

test('a window that has already reset shows no figure @ phone width', async ({ page }) => {
  // The normal case, not an edge one. The number comes from Claude Code's
  // status line, which belongs to a terminal — so a console driven from a phone
  // is routinely looking at a reading taken hours and one window ago. 28% then
  // describes a five-hour window that no longer exists.
  await mockRunner(page);
  await page.goto('/');
  const strip = page.locator('.usage');
  await strip.waitFor();
  await expect(strip).toContainText('reset since');
  await expect(strip, 'a figure from a window that has gone').not.toContainText('28%');
});

test('the list says nothing about which machine it is @ phone width', async ({ page }) => {
  // "this Mac" was true of every session on the list and news to nobody. What
  // replaced it is nothing about a session: everything the ⋮ menu offers acts on
  // one, and on the list there is no session on screen.
  //
  // Narrowed TWICE now, and each time for the same reason: the assertion
  // was stronger than the sentence above it. First it said the bar held no
  // button at all, which stopped being true when keeping the screen on was
  // offered here. Then it said the bar held no MENU, which stopped being true
  // when a screen arrived that is about the whole corpus rather than any
  // session. The invariant is the sentence — *nothing aimed at a session that
  // has not been chosen yet* — so that is what this now checks, item by item.
  await mockRunner(page);
  await page.goto('/');
  await page.getByText('decode').first().waitFor();
  await expect(page.locator('ui-scaffold')).not.toContainText('Mac');
  // Nowhere above the list, so nothing to go up to.
  await expect(page.locator('ui-scaffold [aria-label="all sessions"]')).toHaveCount(0);

  await page.getByRole('button', { name: 'Menu' }).click();
  for (const aimed of ['Details', 'Go to…', 'Tasks', 'Rename', 'Stop']) {
    await expect(
      page.getByRole('menuitem', { name: aimed }),
      `${aimed} acts on a session, and none is chosen here`,
    ).toHaveCount(0);
  }
});

test('reader — the survey is a screen you go to, not a strip in the way @ phone width', async ({
  page,
}, testInfo) => {
  await mockRunner(page);
  await page.goto('/');

  // Absent from the list, and that is the assertion. It was a strip above
  // the sessions; the list answers *what should I do now* and this answers
  // *what has been done, ever*, which is a question you go looking for.
  await expect(page.locator('app-reading-view')).toHaveCount(0);

  await page.getByRole('button', { name: 'Menu' }).click();
  await page.getByRole('menuitem', { name: 'Reader' }).click();

  // The claim and its ceiling, both drawn.
  await page.getByText('99.2%').waitFor();
  await page.getByText('4.6%').waitFor();

  // Every shape, not the head of them. The strip showed five, which was
  // right on the way past and wrong on a page somebody navigated to — the tail
  // is where `not understood` lives, and it qualifies everything above it.
  await page.getByText('run a program on another machine (no shell)').waitFor();
  await page.getByText('not understood').waitFor();

  // One number from each section, because a missing field renders as
  // nothing and asserting only on prose passes over a blank page.
  await page.getByText('411,965').waitFor();
  await page.getByText('37,747').waitFor();
  await page.getByText('2,747').waitFor();
  await page.getByText('k3s').waitFor();
  await page.getByText('294,504', { exact: false }).waitFor();

  // The section that can never be worked, and a number from it. It was
  // the biggest name on the queue above until it was split out, and drawing it
  // without a figure would leave the split invisible — `| number` renders an
  // absent field as empty rather than as an error, which is how the first
  // version of this page shipped half-blank with every assertion green.
  await page.getByText('2,475', { exact: false }).waitFor();
  // The ROW, not the word. `probe` is named in the prose above the list
  // as well, so a bare text locator resolves to two elements and fails strict
  // mode — which is the locator being right, not the page being wrong.
  await page.getByRole('listitem').filter({ hasText: '418probe' }).waitFor();

  // Its own bar, with a way back.
  await expect(page.getByRole('heading', { name: 'Reader' })).toBeVisible();
  await page.getByRole('button', { name: 'all sessions' }).click();
  await page.locator('.session').first().waitFor();

  await expectIconFontLoaded(page);
  await expectCleanLayout(page, testInfo);
});

test('reader — an unmined survey says so on its own screen @ phone width', async ({
  page,
}, testInfo) => {
  await mockRunner(page);
  await page.route('**/api/reading', (r) => r.fulfill({ status: 404, body: '' }));
  await page.goto('/reader');

  await page.getByText('No survey has been mined here').waitFor();
  await expect(page.locator('.chart')).toHaveCount(0);
  await expectCleanLayout(page, testInfo);
});

test('usage — the week bar marks the days and where the clock is @ phone width', async ({
  page,
}, testInfo) => {
  // A week is measured in waking hours, so where its marks fall depends on the
  // time of day: pinned to a Wednesday at 14:00.
  await page.clock.setFixedTime(new Date(2026, 8, 23, 14));
  await mockRunner(page);
  await page.goto('/');

  const week = page.locator('.window', { hasText: 'Week' }).locator('.level');
  await week.waitFor();

  // Six ticks, not seven. They are the boundaries INSIDE a seven-day
  // window; the bar's own two ends already mark the outer ones, and a seventh
  // drawn at 100% would sit under the edge and read as a rendering fault.
  await expect(week.locator('.day')).toHaveCount(6);

  // The window runs Friday 20:00 to Friday 20:00: 98 waking hours. The first
  // boundary, Saturday 20:00, is 14 of them in.
  const first = await week
    .locator('.day')
    .first()
    .evaluate((el: HTMLElement) => el.style.left);
  expect(Number.parseFloat(first)).toBeCloseTo((14 / 98) * 100, 1);

  // The clock, placed at the reading, taken 4h ago at 10:00: 60 waking hours
  // in. 66% spent against 61% gone is ahead enough to be amber.
  const clock = week.locator('.clock');
  await expect(clock).toHaveCount(1);
  const at = await clock.evaluate((el: HTMLElement) => el.style.left);
  expect(Number.parseFloat(at)).toBeCloseTo((60 / 98) * 100, 1);
  await expect(page.locator('.window', { hasText: 'Week' }).locator('.pct.over')).toHaveCount(1);

  // The five-hour row gets no day ticks, and asserting that is the point:
  // a window with no unit a person tracks would gain noise, not information.
  const short = page.locator('.window', { hasText: '5 hours' }).locator('.level');
  await expect(short.locator('.day')).toHaveCount(0);

  await expectIconFontLoaded(page);
  await expectCleanLayout(page, testInfo);
});

test('the list shows what this phone last saw at once while the Mac is slow @ phone width', async ({
  page,
}, testInfo) => {
  // A phone that has just woken: the list as last seen, under a bar, until the
  // runner answers — not an empty page.
  await mockRunner(page);
  await page.goto('/');
  const card = page.locator('.session').first();
  await expect(card, 'the list was never read in the first place').toBeVisible();
  await page.waitForTimeout(1000);

  let release: () => void = () => undefined;
  const answered = new Promise<void>((resolve) => (release = resolve));
  await page.route('**/api/state', async (r) => {
    await answered;
    await r.fallback();
  });
  await page.goto('/');
  await expect(card, 'the kept list was not drawn while the Mac was slow').toBeVisible({
    timeout: 1500,
  });
  await expect(page.locator('.catching-up')).toBeVisible();
  await expectCleanLayout(page, testInfo, { allow: BUSY_BAR });

  release();
  await expect(page.locator('.catching-up'), 'the copy outlived the answer').toHaveCount(0);
});
