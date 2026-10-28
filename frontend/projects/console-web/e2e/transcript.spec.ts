import {
  BUSY_BAR,
  ENDED,
  LANDMARKS,
  LATE,
  LONG_ANSWER,
  NEXT,
  RUNNING,
  STATE,
  TRANSCRIPT,
  distanceFromTheEnd,
  expect,
  expectCleanLayout,
  expectClocksOnTheirLine,
  expectComposerFillsTheWidth,
  expectIconFontLoaded,
  expectIconsCentred,
  expectNoPinnedOverlap,
  expectSendAlignsWithTheBox,
  expectThumbTargets,
  handControlOfTheStream,
  mockRunner,
  mutter,
  openTools,
  routeStream,
  say,
  test,
  thumb,
  tinyPng,
  type Earlier,
  type Overview,
  type Returned,
  type Summary,
  type Timed,
} from './console';

test.use({ serviceWorkers: 'block' });

test('transcript — tool arguments and a fixed composer @ phone width', async ({
  page,
}, testInfo) => {
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  // The failed shell call is the widest thing on the page; wait for it rather
  // than for the first paint, or the checks run against half a transcript.
  await openTools(page);
  await page.getByText('verified_cli').first().waitFor();

  // The codes, not the colour. A great deal of what the fleet runs writes
  // SGR — `cargo`, `vitest`, `eslint` — and with nothing reading it the ESC byte
  // is invisible while the rest is not, so the row read
  // `[1;31merror[0m: unknown flag`. Asserted on the escape sequence rather than
  // on a colour: what went wrong was text nobody could read, and that it is now
  // red is a matter for the render.
  await expect(page.getByText('[1;31m')).toHaveCount(0);
  await expect(page.locator('.returned.ansi .ansi-fg-1').first()).toBeVisible();
  await expectCleanLayout(page, testInfo, { allow: BUSY_BAR });
  await expectNoPinnedOverlap(page);
  await expectIconsCentred(page);
  await expectClocksOnTheirLine(page);
  await expectComposerFillsTheWidth(page);
  await expectSendAlignsWithTheBox(page);
});

test('go to — a long conversation is reachable by landmark @ phone width', async ({
  page,
}, testInfo) => {
  // The screen this exists for. A transcript here runs to hundreds of
  // megabytes and pages back 400 events at a time, so anything an hour old is a
  // hundred taps away. This sheet is the way back, which makes its rows the one
  // place in the app where a pasted path has to fit on a phone.
  await mockRunner(page);
  await page.route('**/api/sessions/*/landmarks', (r) => r.fulfill({ json: LANDMARKS }));
  // The page a jump lands on. `mockRunner` answers every other GET with `[]`,
  // which is not the shape this one returns — an empty array would leave the
  // view with no entries and nothing to say why.
  await page.route('**/api/sessions/*/earlier*', (r) =>
    r.fulfill({
      // A cursor of 0 — the start of the file — so the infinite-scroll observer
      // stops asking. A non-zero cursor with a constant answer pages the same
      // event in forever, which is a fixture artefact and not what the app does.
      json: {
        events: [{ kind: 'text', text: 'what was said back then' }],
        from: 0,
      } satisfies Earlier,
    }),
  );
  await page.goto(`/s/${RUNNING.id}`);
  await page.getByRole('button', { name: /what to do with/ }).click();
  await page.getByRole('menuitem', { name: 'Go to…' }).click();

  const sheet = page.locator('mat-bottom-sheet-container');
  await expect(sheet).toBeVisible();
  await sheet.getByText('that is the one, thanks').waitFor();
  // Grouped by day, newest day first — the thing somebody wants back is far
  // more often this afternoon's than last week's.
  await expect(sheet.locator('.day')).toHaveText(['Today', 'Yesterday']);
  // A compaction is a place rather than something said, so it carries the words
  // the runner does not send.
  await expect(sheet.getByText('the conversation was cut here')).toBeVisible();

  await expectIconFontLoaded(page);
  await expectCleanLayout(page, testInfo, { root: 'mat-bottom-sheet-container' });
  // Scoped to the sheet: it sits OVER the transcript by design, so an unscoped
  // check reports the conversation behind it overlapping every row.
  // Every row here is pressable, unlike the task sheet where most are not — so
  // the whole row is the target and it may not be shaved.
  await expectThumbTargets(page);

  // And the state a jump leaves behind, which is the part that can mislead.
  // Detached, the transcript does not grow and the header's "working" is a
  // reading from before — so the banner is the only thing on screen telling the
  // truth about what is being looked at.
  await sheet.getByText('that is the one, thanks').click();
  const adrift = page.locator('.deaf.adrift');
  await expect(adrift, 'nothing says this is not the live conversation').toBeVisible();
  await expect(adrift.getByText('Looking back.')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Back to now' })).toBeVisible();
  await expectCleanLayout(page, testInfo);
  await expectThumbTargets(page);
});

test('a command waiting for the turn says so, and can be taken back @ phone width', async ({
  page,
}, testInfo) => {
  // A slash command written to a working session is not run: the CLI parks it
  // as a `queued_command` and hands it to the model as words. The runner holds
  // it instead, and the conversation says so where the command will appear.
  await mockRunner(page);
  const id = RUNNING.id;
  let held = true;
  const working = (): Summary => ({
    ...RUNNING,
    working: true,
    messages: held ? [{ id: 'h-1', text: '/compact', at: Date.now(), stands: 'held' }] : [],
  });
  await page.route('**/api/state', (r) =>
    r.fulfill({ json: { ...STATE, sessions: [working(), ENDED] } satisfies Overview }),
  );
  let asked: Record<string, unknown> | undefined;
  await page.route(`**/api/sessions/${id}/withdraw`, (r) => {
    asked = r.request().postDataJSON() as Record<string, unknown>;
    held = false;
    return r.fulfill({
      json: { text: '/compact', pictures: [], session: working() } satisfies Returned,
    });
  });
  await page.goto(`/s/${id}`);

  const waiting = page.locator('li.entry', { hasText: 'runs when this turn ends' });
  await waiting.waitFor();
  await expect(waiting).toContainText('/compact');
  await expectCleanLayout(page, testInfo, { allow: BUSY_BAR });
  await expectThumbTargets(page);

  await waiting.getByRole('button', { name: 'take back' }).click();
  await expect.poll(() => asked?.['id']).toBe('h-1');
  // Back in the box, to be sent again or not.
  await expect(page.locator('app-composer textarea')).toHaveValue('/compact');
  await expect(page.locator('li.entry', { hasText: 'runs when this turn ends' })).toHaveCount(0);
});

test('what is being written survives leaving the conversation @ phone width', async ({ page }) => {
  // Reported from the phone: typed words and a chosen picture were lost going up
  // to the list and coming back. The picture is the expensive half — it cost a
  // scale of a phone photograph — and the words are the half nobody wants to
  // type twice on a phone.
  await mockRunner(page);
  const first = RUNNING.id;
  const second = ENDED.id;
  await page.goto(`/s/${first}`);
  await page.locator('.composer').waitFor();
  await page.locator('.picker').setInputFiles(tinyPng());
  await page.locator('.chosen').waitFor();
  await page.locator('.composer textarea').fill('half a thought about this');

  // Up to the list, which destroys the view — the actual reported action, and
  // not `goto`, which would reload the page and prove something else.
  await page.locator('[aria-label="all sessions"]').click();
  await expect(page.locator('.composer')).toHaveCount(0);

  // Through the OTHER conversation on the way back, because a draft that was
  // global rather than per session would pass the simple there-and-back and
  // still put one conversation's words into another.
  await page.goto(`/s/${second}`);
  await page.locator('.composer').waitFor();
  await expect(page.locator('.composer textarea')).toHaveValue('');
  await expect(page.locator('.chosen')).toHaveCount(0);

  await page.goto(`/s/${first}`);
  await expect(page.locator('.composer textarea')).toHaveValue('half a thought about this');
  await expect(page.locator('.chosen .thumb')).toBeVisible();
  await expect(page.locator('.chosen .about')).toContainText('2×4');
  // Visible is not drawn. A held picture's preview is an object URL, which
  // belongs to the document that made it and is dead in the next one — a revived
  // draft carrying one would show a broken image that still passes every
  // assertion above. `naturalWidth` is the browser saying it decoded the bytes.
  await expect
    .poll(() =>
      page.locator('.chosen .thumb').evaluate((img) => (img as HTMLImageElement).naturalWidth),
    )
    .toBe(2);
});

test('a finger on the transcript stops it being pulled to the end @ phone width', async ({
  page,
}) => {
  // Reported from the phone. A session writing its answer pulled the view
  // to the end on every delta, including while the reader had a thumb on the
  // glass reading the sentence as it arrived — they had not scrolled, so they
  // were still pinned, and being pinned is exactly what moved the view.
  //
  // The rules themselves are unit-tested in `following.spec.ts`, which is where
  // the numbers live. What only a browser can answer is whether the touch ever
  // reaches them: the handlers are template bindings, and a binding that is not
  // there fails silently.
  await mockRunner(page);
  await routeStream(page, (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: (
        [
          { kind: 'started', model: 'claude-opus-5[1m]', cwd: '/home/example/Code', tools: 14 },
          ...Array.from({ length: 60 }, (_, n): Timed => ({
            kind: 'text',
            text: `paragraph ${n} of an answer long enough to scroll.\n\n`,
            at: NEXT,
          })),
        ] satisfies Timed[]
      )
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.goto(`/s/${RUNNING.id}`);
  const list = page.locator('.transcript');
  await list.waitFor();

  // Opened at the newest, which is the state the defect happens in.
  const atEnd = () =>
    list.evaluate((box) => Math.round(box.scrollHeight - box.scrollTop - box.clientHeight));
  await expect.poll(atEnd).toBeLessThan(4);
  const before = await list.evaluate((box) => box.scrollTop);

  // A finger goes down and the window shrinks under it — the same thing a
  // growing answer does to the distance from the end, and the one way to make
  // it happen in a harness that hands the transcript over in a single chunk.
  await list.dispatchEvent('touchstart');
  await page.setViewportSize({ width: 412, height: 420 });

  // Wait for the resize to have been acted on, then assert. The claim here
  // is that something did NOT happen, and there is no event for that — so the
  // window has to be given its chance first. `expect.poll(...).toBe(before)`
  // alone is worse than useless: `before` is the current value, so it passes on
  // the first poll, ahead of the observer that would have moved it. Written that
  // way, this test passed with the `touchstart` binding deleted.
  await expect.poll(() => list.evaluate((box) => box.clientHeight)).toBeLessThan(420);
  await page.waitForTimeout(250);

  expect(await list.evaluate((box) => box.scrollTop), 'not pulled anywhere').toBe(before);
  expect(await atEnd(), 'left behind the end, which is where the reader put it').toBeGreaterThan(
    100,
  );

  // And letting go catches up, rather than leaving the transcript frozen for the
  // life of the page.
  await list.dispatchEvent('touchend');
  await expect.poll(atEnd).toBeLessThan(4);
});

test('opening a session from the list lands at the newest message @ phone width', async ({
  page,
}) => {
  // The path somebody actually takes, and a different one from the tests
  // below: they drive a stream into a page that is already open, where this
  // navigates to a component that does not exist yet and is handed a whole
  // transcript at once. A resumed conversation is replayed from its last 400
  // events, so a view that opened at the top would open a hundred turns behind
  // the present — and read as the page being broken rather than as a scroll
  // position.
  await mockRunner(page);
  await routeStream(page, (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: (
        [
          ...TRANSCRIPT,
          // Enough afterwards to put the end of it well past a phone screen.
          ...Array.from({ length: 25 }, (_, n): Timed => ({
            kind: 'prompt',
            text: `Follow-up ${n + 1}: check the next journey and report what moved.`,
            at: NEXT,
          })),
          { kind: 'text', text: 'THE NEWEST THING SAID' },
        ] satisfies Timed[]
      )
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.goto('/');
  // Tapped from the list, not typed as a URL: opening a session is a navigation
  // into a component that is built fresh, and that is the case at issue.
  await page.locator('.session').first().click();
  await page.getByText('THE NEWEST THING SAID').waitFor();
  await page.evaluate(
    () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))),
  );

  expect(await distanceFromTheEnd(page), 'opened behind the newest message').toBeLessThan(4);
  // And the newest message is actually on screen, not merely scrolled to.
  await expect(page.getByText('THE NEWEST THING SAID')).toBeInViewport();
});

test('scrolling to the top fetches what came before it @ phone width', async ({ page }) => {
  // No control to press. The seed is a page, not the conversation, and
  // reaching the top is the request, so what this measures is
  // that a reader who scrolls up gets more — and keeps getting it, which is the
  // half a single fetch would not show.
  let asked = 0;
  await mockRunner(page);
  // The seed carries the byte offset it began at, and that — not the number of
  // entries — is what says the file holds more. Without it there is nothing
  // above the transcript to reach.
  // A finite body ends the stream, and the browser reconnects about 3s later.
  // The runner resumes such a reader from its last event id with nothing new;
  // served the seed again, the page jumped back to the newest message and its
  // cursor back to 5000, and asked again (7 of 120 under load, every one on a
  // second connection).
  let streams = 0;
  await page.route('**/api/sessions/*/events', (r) =>
    ++streams > 1
      ? r.fulfill({ contentType: 'text/event-stream', body: 'retry: 600000\n\n' })
      : r.fulfill({
          contentType: 'text/event-stream',
          body: (
            [
              ...TRANSCRIPT,
              // Several screens of it, so that "at the newest message" is genuinely
              // far from the top. The seed is a page and a page is 400 events; a
              // fixture only a little taller than the screen puts the mark inside the
              // 400px of prefetch margin at every scroll position, and the check
              // below would then be asserting the margin does not exist.
              ...Array.from({ length: 40 }, (_, n): Timed => ({
                kind: 'prompt',
                text: `Message ${n + 1} of the page that was seeded when this view opened.`,
                at: NEXT,
              })),
              { kind: 'joined', earlier: TRANSCRIPT.length + 40, from: 5000, restarted: false },
            ] satisfies Timed[]
          )
            .map((event) => `data: ${JSON.stringify(event)}\n\n`)
            .join(''),
        }),
  );
  await page.route('**/api/sessions/*/earlier?*', async (r) => {
    asked += 1;
    const page_ = asked;
    await r.fulfill({
      json: {
        // Walks backwards towards the start of the file, and reaches it on the
        // third answer — `from: 0` is what tells the client there is no more.
        from: page_ >= 3 ? 0 : 5000 - page_ * 1000,
        events: Array.from({ length: 12 }, (_, n): Timed => ({
          kind: 'prompt',
          text: `Older page ${page_}, message ${n + 1} — something said earlier this morning.`,
          at: LATE,
        })),
      } satisfies Earlier,
    });
  });
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('.transcript').waitFor();
  await expect(page.locator('.earlier')).toHaveCount(1);
  // The control it replaces is gone, not merely hidden.
  await expect(page.getByRole('button', { name: 'earlier messages' })).toHaveCount(0);

  // Nothing is asked for while the reader is at the newest message.
  expect(asked, 'fetched without being anywhere near the top').toBe(0);

  // One page per arrival at the top, not the whole file. Landing a page
  // and holding the reader's place puts the mark back out of view, which is the
  // point: "starting small" would mean nothing if reaching the top once
  // unspooled a 1.4 GB transcript. So each fetch here is a fresh journey to the
  // top, which is what a reader travelling backwards through a morning does.
  // Throws on a missing container: a silent `if (box)` would read as a scroll
  // that happened, and the failure would surface later at the poll below.
  const toTheTop = async () => {
    await page.locator('.transcript').waitFor();
    const scrolled = await page.evaluate(() => {
      const box = document.querySelector('.transcript');
      if (!box) return null;
      box.scrollTop = 0;
      return box.scrollTop;
    });
    if (scrolled === null) {
      throw new Error('toTheTop: .transcript is not in the DOM — nothing was scrolled');
    }
  };

  // A failure here has been seen twice at night with the page at the top and
  // nothing asked, and has not reproduced since. So a timeout says
  // what the page held when it gave up: how far the mark was from the prefetch
  // margin, whether a read was in flight, and every request the session made.
  const opened = Date.now();
  const requests: string[] = [];
  page.on('request', (sent) => {
    const url = new URL(sent.url());
    if (url.pathname.startsWith('/api/sessions/')) {
      requests.push(
        `+${Date.now() - opened}ms ${url.pathname.split('/').pop() ?? ''}${url.search}`,
      );
    }
  });
  const held = (): Promise<string> =>
    page.evaluate(() => {
      const box = document.querySelector('.transcript');
      const mark = document.querySelector('.earlier');
      if (!box) return 'no .transcript';
      const frame = box.getBoundingClientRect();
      const at = mark?.getBoundingClientRect();
      return JSON.stringify({
        scrollTop: box.scrollTop,
        scrollHeight: box.scrollHeight,
        clientHeight: box.clientHeight,
        markFromTop: at ? Math.round(at.top - frame.top) : null,
        markInMargin: at ? at.bottom >= frame.top - 400 && at.top <= frame.bottom : null,
        markSays: mark?.textContent?.trim() ?? null,
        rows: box.querySelectorAll('li').length,
      });
    });
  const reaches = async (count: number): Promise<void> => {
    try {
      await expect.poll(() => asked, { timeout: 5000 }).toBe(count);
    } catch (failed) {
      throw new Error(
        `${failed instanceof Error ? failed.message : String(failed)}\n\n` +
          `what the page held: ${await held()}\nrequests: ${requests.join(', ')}`,
      );
    }
  };

  await toTheTop();
  await reaches(1);
  // The dash matters: without it this also matches messages 10 to 12.
  await expect(page.getByText('Older page 1, message 1 —')).toHaveCount(1);

  await toTheTop();
  await reaches(2);
  await expect(page.getByText('Older page 2, message 1 —')).toHaveCount(1);

  // The third answer says `from: 0` — the start of the file — and the mark that
  // asks for more goes with it.
  await toTheTop();
  await reaches(3);
  await expect(page.locator('.earlier')).toHaveCount(0);

  // And no amount of scrolling asks again for a conversation that has none.
  await toTheTop();
  await page.waitForTimeout(300);
  expect(asked, 'kept asking past the start of the file').toBe(3);
});

test('an answer does not pay for the newlines between its blocks @ phone width', async ({
  page,
}) => {
  // The container preserves whitespace and the content is markup. `.entry`
  // sets `pre-wrap` so a message keeps the line breaks somebody typed; `marked`
  // puts a newline between every block it emits, and each one rendered as a real
  // line break on top of that block's own margin. Measured before the fix:
  // 44.8px, 52.8px, 44.8px and 40.0px where the margins ask for 16, and an
  // answer 390px tall that should be 224 — three quarters of a phone screen
  // given to nothing, on the most common thing this page renders.
  //
  // Asserted as a ceiling rather than an equality: the margins are a design
  // choice and may move, and what this is guarding against is a gap that has
  // nothing to do with them.
  await mockRunner(page);
  await routeStream(page, (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: (
        [
          { kind: 'started', model: 'x', cwd: '/home/example/Code/memview', tools: 1 },
          { kind: 'text', text: '## A heading\n' },
          { kind: 'text', text: '\nA paragraph of prose.\n' },
          { kind: 'text', text: '\n## Another heading\n' },
          { kind: 'text', text: '\nAnd a second paragraph.\n' },
          { kind: 'turn', cost_usd: 0, turns: 1, duration_ms: 1, at: NEXT },
        ] satisfies Timed[]
      )
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('.body').first().waitFor();

  const gaps = await page.evaluate(() => {
    const body = document.querySelector('.body');
    if (!body) return [];
    const kids = [...body.children];
    const found: { between: string; gap: number }[] = [];
    for (let i = 0; i < kids.length - 1; i++) {
      const top = kids[i];
      const under = kids[i + 1];
      if (!top || !under) continue;
      const above = top.getBoundingClientRect();
      const below = under.getBoundingClientRect();
      found.push({
        between: `${top.tagName}→${under.tagName}`,
        gap: Math.round((below.top - above.bottom) * 10) / 10,
      });
    }
    return found;
  });
  expect(gaps.length, 'the answer rendered as one block, so nothing was measured').toBeGreaterThan(
    1,
  );
  expect(
    gaps.filter((g) => g.gap > 24),
    'blocks pushed apart by more than their margins',
  ).toEqual([]);
});

test('a table keeps the alignment its author wrote @ phone width', async ({ page }) => {
  // Only a rendered page can answer this. `marked` emits `align="right"`
  // and Angular's sanitiser keeps the attribute — both testable in jsdom, and
  // both were already true when every right-aligned column came out left. A
  // presentational attribute loses to any CSS rule, and a blanket
  // `text-align: left` on `th, td` was throwing all of it away.
  await handControlOfTheStream(page);
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('.transcript').waitFor();
  await say(
    page,
    { kind: 'text', text: '| l | m | r | n |\n|:---|:---:|---:|---|\n| 1 | 2 | 3 | 4 |\n' },
    1,
  );

  const cells = await page.evaluate(() =>
    [...document.querySelectorAll('.body table td')].map((cell) =>
      // Chromium reports alignment that came from the `align` ATTRIBUTE as
      // `-webkit-left` / `-webkit-center` / `-webkit-right`, and alignment from
      // a stylesheet as plain `left`. The prefix is not noise to normalise away
      // and forget — it is the evidence that the table's own instruction
      // survived the cascade, which is the whole question here.
      getComputedStyle(cell).textAlign.replace('-webkit-', ''),
    ),
  );
  // The fourth column asked for nothing, and left is what that should mean.
  expect(cells, 'the cascade is overriding the table').toEqual(['left', 'center', 'right', 'left']);
});

test('a task list says which of its items are done @ phone width', async ({ page }) => {
  // The rendered half of the same pair: the mark has to be on screen, and the
  // bullet that would sit beside it has to be gone.
  await handControlOfTheStream(page);
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('.transcript').waitFor();
  await say(page, { kind: 'text', text: '- [x] shipped\n- [ ] not yet\n' }, 1);

  const items = await page.evaluate(() =>
    [...document.querySelectorAll('.body li.task')].map((li) => ({
      text: (li.textContent ?? '').trim(),
      marker: getComputedStyle(li).listStyleType,
    })),
  );
  expect(items).toEqual([
    { text: '☑ shipped', marker: 'none' },
    { text: '☐ not yet', marker: 'none' },
  ]);
});

test('the transcript keeps following while the reader is at the end @ phone width', async ({
  page,
}) => {
  // The contract this pins: a reader at the end stays at the end as the session
  // talks, without touching anything. It is the normal way this page is used —
  // watching a session work — and nothing measured it, so "it stopped following"
  // had no answer but somebody's memory of it.
  await handControlOfTheStream(page);
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('.transcript').waitFor();

  await say(page, { kind: 'prompt', text: 'go on then', at: NEXT }, 1);
  expect(await distanceFromTheEnd(page), 'not at the end to begin with').toBeLessThan(4);

  // One entry taller than the screen, then a second: following has to survive
  // both the first arrival and the steady state after it.
  await say(page, { kind: 'text', text: LONG_ANSWER }, 2);
  expect(await distanceFromTheEnd(page), 'stopped following after a long answer').toBeLessThan(4);

  await say(page, { kind: 'text', text: LONG_ANSWER }, 3);
  expect(await distanceFromTheEnd(page), 'stopped following on the next answer').toBeLessThan(4);
});

test('the transcript does not yank a reader who has scrolled back @ phone width', async ({
  page,
}) => {
  // The other half of the same contract, and the reason it is conditional at
  // all: dragging the view down while somebody is reading back through the
  // morning is worse than not following.
  await handControlOfTheStream(page);
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('.transcript').waitFor();
  await say(page, { kind: 'text', text: LONG_ANSWER }, 1);

  // With the wheel, not by writing `scrollTop`. Setting the position from
  // a script is exactly what the browser itself does when it anchors, and the
  // page now declines to read that as a decision — see [[SessionView.handled]].
  // A test that moved the view the way no reader can would have gone on passing
  // while the real gesture was broken, and it was the only check standing behind
  // "does not yank a reader who has scrolled back".
  await page.locator('.transcript').hover();
  await page.mouse.wheel(0, -20000);
  await page.evaluate(() => new Promise((done) => requestAnimationFrame(done)));
  const away = await distanceFromTheEnd(page);
  expect(away, 'the scroll back did not take').toBeGreaterThan(200);

  await say(page, { kind: 'text', text: LONG_ANSWER }, 2);
  const after = await distanceFromTheEnd(page);
  // Further from the end than before, because the transcript grew underneath a
  // reader who did not move — and emphatically not back at the bottom.
  expect(after, 'yanked the reader to the end').toBeGreaterThan(away);
});

test('the transcript keeps following through a thumb resting on it @ phone width', async ({
  page,
}) => {
  // Every following check above moves the view with a wheel or not at all; this
  // one has a finger on the glass. Deciding per scroll event while held, a thumb
  // resting still drifts far enough to end following, and a live session reads
  // as a dead page.
  await handControlOfTheStream(page);
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('.transcript').waitFor();
  await say(page, { kind: 'text', text: LONG_ANSWER }, 1);
  expect(await distanceFromTheEnd(page), 'not at the end to begin with').toBeLessThan(4);

  // Watch how far the view actually travels, rather than trusting the
  // gesture. Chromium eats an unknown amount of the finger's movement as
  // gesture slop, so a request to drag 30px can move the view by 30 or by
  // nothing — and "by nothing" would leave this test passing without ever having
  // exercised a hold. The synthesized gesture is atomic, so the furthest point
  // has to be collected as it happens.
  const furthest = async () => page.evaluate(() => (window as unknown as { __min: number }).__min);
  await page.evaluate(() => {
    const el = document.querySelector('.transcript')!;
    const w = window as unknown as { __min: number };
    w.__min = el.scrollTop;
    el.addEventListener('scroll', () => (w.__min = Math.min(w.__min, el.scrollTop)));
  });
  const before = await page.evaluate(() => document.querySelector('.transcript')!.scrollTop);

  await thumb(page, 45);

  const moved = before - (await furthest());
  // The band is the test. Only a movement ABOVE `SLACK` (16) and BELOW
  // `SLOP` (40) can tell the rule from its absence: under 16 nothing would unpin
  // either way. Chromium eats some of a drag as gesture slop, so these bounds
  // fail loudly if the movement drifts out of the band.
  expect(moved, 'under SLACK — the old engine would have forgiven this too').toBeGreaterThan(16);
  expect(moved, 'over SLOP — that is a scroll back, which is the test below').toBeLessThan(40);

  // `SLACK`, not the 4px the wheel checks use. A real touch gesture leaves
  // a few pixels of momentum behind it where a wheel stops dead — measured at 5.
  // What matters is that the view is still inside what the engine itself counts
  // as the end, so the next thing the session writes is followed; that is the
  // line below, and it is the strict one.
  expect(await distanceFromTheEnd(page), 'a resting thumb ended following').toBeLessThan(16);
  await say(page, { kind: 'text', text: LONG_ANSWER }, 2);
  expect(await distanceFromTheEnd(page), 'did not follow after the thumb lifted').toBeLessThan(4);
});

test('the transcript picks a reader up again when they scroll back to the end @ phone width', async ({
  page,
}) => {
  // Reported from the phone: reaching the very end by hand and sitting
  // there, with the session still writing, left the page not following — a gap of
  // a pixel and still declining to write, then widening as the conversation grew
  // under a reader who had not moved. Once away, every move is
  // theirs, INCLUDING the one that comes back; the unit tests say so and nothing
  // said it through a real gesture.
  await handControlOfTheStream(page);
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('.transcript').waitFor();
  await say(page, { kind: 'text', text: LONG_ANSWER }, 1);

  await thumb(page, 300);
  const away = await distanceFromTheEnd(page);
  expect(away, 'the scroll back did not take').toBeGreaterThan(200);

  // Back down, far enough that the box clamps at its own end rather than needing
  // the gesture to land on a pixel — and NO FURTHER.
  //
  // This asked for a flat 600px and that is what made the test flaky. What
  // a drag asks for beyond the end is banked by desktop Chromium and paid out
  // the moment something makes the box taller, as a decaying series of scroll
  // events. The first of them is 20px against `SLACK = 16`, so if it lands
  // before the follow, `following.ts` reads it as the reader leaving and unpins
  // for good — and the assertion below fails with the whole answer's worth of
  // gap. Measured, holding everything else still:
  //
  //     asked   overshoot   scroll events   drift
  //      320px       35px         0            0
  //      600px      315px        26         ~170px
  //     1200px      915px        30         ~371px
  //
  // The console is not at fault and must not be changed for this. Driven
  // on the Pixel 9 through real Android touch — a 700ms drag and a 90ms fling,
  // both asking for well past the end — the phone's WebView banks nothing:
  // growing the transcript by 2500px afterwards produced ZERO scroll events,
  // twice. The banking is desktop Chromium under `Input.dispatchTouchEvent`, so
  // it is this file's problem and only this file's.
  //
  // Asked for from the gap rather than as a constant, because the margin is what
  // matters: enough to clamp through the slop Chromium eats, not enough to bank.
  await thumb(page, -(away + 40));
  expect(await distanceFromTheEnd(page), 'did not reach the end').toBeLessThan(16);

  await say(page, { kind: 'text', text: LONG_ANSWER }, 2);
  expect(
    await distanceFromTheEnd(page),
    'came back to the end and was not picked up again',
  ).toBeLessThan(4);
});

test('the transcript stops following when the finger really scrolled back @ phone width', async ({
  page,
}) => {
  // The other side of the same gesture: a real scroll back is not overridden.
  // A hold is forgiven precisely because a drag is not, so forgiving one without
  // the other is not a fix, it is following that cannot be stopped by hand.
  await handControlOfTheStream(page);
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('.transcript').waitFor();
  await say(page, { kind: 'text', text: LONG_ANSWER }, 1);

  await thumb(page, 400);
  const away = await distanceFromTheEnd(page);
  expect(away, 'the scroll back did not take').toBeGreaterThan(200);

  await say(page, { kind: 'text', text: LONG_ANSWER }, 2);
  expect(await distanceFromTheEnd(page), 'yanked the reader to the end').toBeGreaterThan(away);
});

test('the transcript follows an answer arriving in deltas @ phone width', async ({ page }) => {
  // The shape a real answer has, and the one the two tests above do not
  // cover: forty small `text` events, none of them waiting for the view to
  // catch up. `follow()` reads a height inside a `requestAnimationFrame` and
  // sets `scrollTop`, and that set fires a scroll event which is what decides
  // whether the reader still counts as at the end — so deltas landing between
  // the two are the case where the page could talk itself out of following.
  await handControlOfTheStream(page);
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('.transcript').waitFor();
  await say(page, { kind: 'prompt', text: 'explain the decoder', at: NEXT }, 1);

  for (let delta = 0; delta < 40; delta++) {
    await mutter(
      page,
      { kind: 'text', text: `Sentence ${delta + 1} of the explanation. ` },
      delta + 2,
    );
  }
  await page.evaluate(
    () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))),
  );
  expect(await distanceFromTheEnd(page), 'lost the end while an answer streamed').toBeLessThan(4);
});

test('the transcript is at the end again after the stream resets @ phone width', async ({
  page,
}) => {
  // A reconnect the runner cannot resume — a console restarted, a tunnel that
  // dropped for long enough — throws the transcript away and rebuilds it. The
  // reader did not ask for that and did not move, so the page has to come back
  // where it was: at the end. This is the likeliest way "it stopped following"
  // happens without anybody touching the screen.
  await handControlOfTheStream(page);
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('.transcript').waitFor();
  await say(page, { kind: 'text', text: LONG_ANSWER }, 1);
  expect(await distanceFromTheEnd(page)).toBeLessThan(4);

  const reset = await page.evaluate(() =>
    (window as unknown as { __resetStream(): boolean }).__resetStream(),
  );
  expect(reset, 'nothing was listening for a reset').toBe(true);
  await say(page, { kind: 'joined', earlier: 1, from: 0, restarted: false }, 1);
  await say(page, { kind: 'text', text: LONG_ANSWER }, 2);
  expect(await distanceFromTheEnd(page), 'left behind by a reconnect').toBeLessThan(4);
});

test('the transcript keeps its end while the composer grows @ phone width', async ({ page }) => {
  // The one thing that moves the end of the transcript without the
  // transcript changing. The composer is `flex: 0 0 auto` above it, so every
  // line typed takes a line off the scrolling region — the reader has not
  // moved, no event has arrived, and the message being answered slides out of
  // sight. `session-view.ts` follows on `visualViewport` resize, which is the
  // soft keyboard; the box growing under a thumb is not that, and nothing else
  // asks.
  await handControlOfTheStream(page);
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('.transcript').waitFor();
  await say(page, { kind: 'text', text: LONG_ANSWER }, 1);
  expect(await distanceFromTheEnd(page)).toBeLessThan(4);

  const box = page.locator('textarea');
  await box.fill('port the gate\nprove it bit-exact\nthen run the golden set\nand report');
  await page.evaluate(
    () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))),
  );
  expect(
    await distanceFromTheEnd(page),
    'typing pushed the newest message out of sight',
  ).toBeLessThan(4);
});

test('closing the keyboard leaves the cursor where it was @ phone width', async ({ page }) => {
  // The keyboard closing resizes the window, and the box's autosize re-measures.
  // With a native placeholder it measures that by writing it into the box and
  // writing the words back, which puts the cursor at the end. Seen on the phone.
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  const box = page.locator('.composer textarea');
  await box.fill('alpha bravo charlie delta echo');
  await box.evaluate((t: HTMLTextAreaElement) => t.setSelectionRange(5, 5));

  const size = page.viewportSize() ?? { width: 412, height: 915 };
  await page.setViewportSize({ width: size.width, height: size.height - 300 });
  await page.waitForTimeout(100);
  await page.setViewportSize(size);
  await page.waitForTimeout(100);

  expect(await box.evaluate((t: HTMLTextAreaElement) => t.selectionStart)).toBe(5);
});

test('the composer grows with what is being typed @ phone width', async ({ page }, testInfo) => {
  // The reason this exists: `rows="1"` meant a long instruction was written
  // through a one-line slit, on the device the console is driven from. jsdom has
  // no layout, so only a real browser can say whether the box actually grew.
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  const box = page.locator('textarea');
  await box.waitFor();
  const oneLine = (await box.boundingBox())!.height;

  await box.fill(
    'port the gate\nprove it bit-exact\nthen run the golden set\nand report what moved',
  );
  const grown = (await box.boundingBox())!.height;
  expect(grown, 'four lines have to be visible as four lines').toBeGreaterThan(oneLine);

  // And it stops. Past a third of the screen the composer is hiding the thing
  // being replied to, which is worse than not seeing the whole instruction.
  await box.fill(Array.from({ length: 40 }, (_, line) => `line ${line}`).join('\n'));
  const capped = (await box.boundingBox())!.height;
  const viewport = page.viewportSize()!.height;
  expect(capped, 'the composer must not eat the transcript').toBeLessThan(viewport / 2);

  await expectCleanLayout(page, testInfo, { allow: BUSY_BAR });
  await expectThumbTargets(page);
});

test('a tool result opens without widening the page @ phone width', async ({ page }, testInfo) => {
  // Unwrapped source, two thousand characters of it, on a 412px screen — the
  // widest single thing the transcript can be asked to hold.
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  await openTools(page);
  const unfold = page.getByRole('button', { name: /characters/ });
  await unfold.waitFor();
  await unfold.click();
  await page.getByText('quantiseLegCost', { exact: false }).first().waitFor();
  await expectCleanLayout(page, testInfo, { allow: BUSY_BAR });
});

test('a run of tool calls is folded into one row @ phone width', async ({ page }) => {
  // Machinery is what a reader scrolls past. A tool call is 115px at phone
  // width and a turn can hold a dozen, so a conversation with any work in it is
  // mostly rows nobody came for, between the two sentences they did.
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  const run = page.locator('.entry.tools .run');
  await run.waitFor();
  await expect(run).toContainText('2 tool calls');
  // The one thing worth saying without opening it.
  await expect(run, 'a failure inside the run is not mentioned').toContainText('1 failed');
  // And the calls themselves are not on the page until asked for.
  await expect(page.getByText('verified_cli')).toHaveCount(0);

  // Still a thumb target, not a 20px summary.
  const box = await run.boundingBox();
  expect(box!.height, 'the run row is under the thumb floor').toBeGreaterThanOrEqual(48);

  await run.click();
  await expect(page.getByText('verified_cli').first()).toBeVisible();
  await run.click();
  await expect(page.getByText('verified_cli')).toHaveCount(0);
});

test('a tool call on its own says which tool it was @ phone width', async ({ page }) => {
  // A path with no verb attached says nothing that matters. `Read` and
  // `Write` on the same file are opposite events, and the argument alone cannot
  // tell them apart — nor can it say a file was deleted rather than looked at.
  //
  // This is a REGRESSION test, and the shape of it is the point. The name
  // was dropped from the standalone row in 65607c7, when the argument grew a
  // parse button and the row became two branches; the FOLDED run kept its copy,
  // so every existing assertion about tool rows still passed while a lone call
  // showed a bare path. Hence the seed below is one call between prose: a run of
  // two folds (`A_RUN`), and a folded row would exercise the wrong markup and
  // pass against the defect.
  await handControlOfTheStream(page);
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('.transcript').waitFor();
  let seq = 0;
  await say(page, { kind: 'said', text: 'Let me look at the gate.' }, ++seq);
  await say(
    page,
    {
      kind: 'tool',
      id: 'lone',
      name: 'Read',
      does: { kind: 'other', shown: '/home/example/gate.ts' },
    },
    ++seq,
  );
  await say(page, { kind: 'said', text: 'That is the one.' }, ++seq);

  const row = page.locator('.entry').filter({ hasText: '/home/example/gate.ts' }).last();
  await expect(row.locator('.tool-name'), 'the row names its tool').toHaveText('Read');
  await expect(row.locator('.tool-arg')).toContainText('gate.ts');
});

test('a running thing says how long it has been running @ phone width', async ({ page }) => {
  // "running" is the same word at four seconds and at forty minutes, and
  // only one of those is worth interrupting. The bar above the composer had the
  // same problem: it says something is happening and could not say for how long,
  // which is the difference between waiting and wondering whether it is stuck.
  await handControlOfTheStream(page);
  await mockRunner(page);
  // Working and blocked are not the same session. The shared fixture holds
  // a standing question, and the strip is deliberately absent while one is on
  // screen — a session waiting on a permission is not one whose output is
  // arriving. See `SessionView.arriving`. This test is about the other case, so
  // it takes the question away rather than asserting a combination the runner
  // cannot produce.
  await page.route('**/api/state', (r) =>
    r.fulfill({
      json: {
        ...STATE,
        sessions: [{ ...RUNNING, waiting: 0 }, ...STATE.sessions.slice(1)],
      } satisfies Overview,
    }),
  );
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('.transcript').waitFor();
  let seq = 0;
  // Stamped two minutes ago, so what is counted from does not depend on how
  // fast this test runs.
  const began = Date.now() - 125_000;
  await say(page, { kind: 'busy', status: 'requesting', at: began }, ++seq);
  await say(
    page,
    {
      kind: 'tool',
      id: 'slow',
      name: 'Bash',
      does: { kind: 'bash', command: 'cargo build' },
      at: began,
    },
    ++seq,
  );

  // The strip is the bar and the clock, and no word. The status the CLI
  // reports — `requesting` — is already in the header at the top of the page,
  // and this sat four inches below it saying the same thing again.
  await expect(page.locator('.doing .lasted')).toHaveText(/2m \d\ds/);
  await expect(page.locator('.doing')).not.toContainText('requesting');
  await expect(page.locator('.head .live'), 'the status belongs to the header').toHaveText(
    'requesting',
  );
  // And on the call itself, which is a separate clock: a background task
  // outlives the turn that started it, and a session can be working with
  // nothing in flight.
  await expect(page.locator('.entry .running').last()).toContainText(/running 2m \d\ds/);
});

test('a run stays folded while it works, and says it is working @ phone width', async ({
  page,
}) => {
  // Not opened automatically. Reading `running > 0` live flickers — a session
  // making one call at a time opens and folds the run with every call — and
  // latching it makes the page depend on whether you had been watching.
  //
  // What the automatic open was for, the summary row does on its face.
  // Before `goto`: it installs its stub through `addInitScript`, which only
  // affects pages loaded after it.
  await handControlOfTheStream(page);
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('.transcript').waitFor();
  let seq = 0;
  for (const event of [
    {
      kind: 'tool',
      id: 'run_a',
      name: 'Bash',
      does: { kind: 'bash', command: 'cargo build --workspace' },
    },
    {
      kind: 'tool',
      id: 'run_b',
      name: 'Bash',
      does: { kind: 'bash', command: 'cargo test --all-features' },
    },
  ]) {
    await say(page, event, ++seq);
  }
  const run = page.locator('.entry.tools .run').last();
  await run.waitFor();
  // The row says work is in flight, which is why folding it is not hiding it.
  await expect(run).toContainText('2 running');
  await expect(page.getByText('cargo test --all-features')).toHaveCount(0);

  // One tap, and the same tap whether the run is live or long finished.
  await run.click();
  await expect(page.getByText('cargo test --all-features')).toBeVisible();

  // And the result that lands next does not close it again. This is the
  // flicker, from the other side: a run opened by hand has to stay open when
  // a running call finishes under the reader.
  await say(page, { kind: 'tool_result', id: 'run_b', ok: true, detail: 'ok' }, ++seq);
  await expect(run, 'the run has finished').not.toContainText('2 running');
  await expect(page.getByText('cargo test --all-features')).toBeVisible();
});

test('one event does not rebuild the rows already on screen @ phone width', async ({ page }) => {
  // The defect this exists for, and it was invisible. `blocks()` wraps each
  // entry in a fresh object on every recompute, so `track block` tracked the
  // wrapper — and every event rebuilt every row, re-rendering the markdown of
  // every message on screen. That is precisely what the comment above that loop
  // claimed to prevent, happening on every delta of every answer. Nothing looked
  // wrong, because the page is correct either way and only wasteful.
  //
  // Stamps a rendered node and watches whether the stamp survives an unrelated
  // event later in the conversation: a property survives only if Angular kept the
  // element instead of making a new one.
  await handControlOfTheStream(page);
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('.transcript').waitFor();
  let seq = 0;
  await say(page, { kind: 'text', text: 'the first thing' }, ++seq);
  await say(page, { kind: 'turn', cost_usd: 0, turns: 1 }, ++seq);

  // Stamp the node that is already on screen. A property survives a re-render
  // only if Angular kept the element rather than making a new one.
  const stamped = await page.evaluate(() => {
    const first = document.querySelector('.transcript .entry');
    if (!first) return false;
    (first as unknown as { __probe?: number }).__probe = 1;
    return true;
  });
  expect(stamped, 'nothing on screen to stamp').toBe(true);

  // An unrelated event at the far end of the conversation.
  await say(page, { kind: 'text', text: 'something later' }, ++seq);

  const survived = await page.evaluate(() => {
    const first = document.querySelector('.transcript .entry');
    return !!(first as unknown as { __probe?: number })?.__probe;
  });
  expect(survived).toBe(true);
});

test('scrolling up by a line stops it following @ phone width', async ({ page }) => {
  // A line is what stops a terminal, a messages app and a chat client
  // following, and a small scroll up must not be pulled back down.
  await handControlOfTheStream(page);
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  const list = page.locator('.transcript');
  await list.waitFor();
  let seq = 0;
  for (let n = 0; n < 20; n++) {
    await say(
      page,
      { kind: 'text', text: `Message ${n}: ` + 'said at some length. '.repeat(6) },
      ++seq,
    );
  }

  // Up by less than a line and a half.
  await list.evaluate((box) => (box.scrollTop -= 40));
  const left = await list.evaluate((box) => box.scrollTop);
  await say(page, { kind: 'text', text: 'AND SOMETHING MORE ARRIVES' }, ++seq);

  expect(await list.evaluate((box) => box.scrollTop), 'not pulled back down').toBe(left);

  // And going back to the end resumes it, because the end is where they are.
  await list.evaluate((box) => (box.scrollTop = box.scrollHeight));
  await say(page, { kind: 'text', text: 'AND MORE AGAIN' }, ++seq);
  const gap = await list.evaluate((box) => box.scrollHeight - box.scrollTop - box.clientHeight);
  expect(gap, 'following again').toBeLessThan(4);
});

test('a seed that arrives in pieces still ends at the end @ phone width', async ({ page }) => {
  await handControlOfTheStream(page);
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('.transcript').waitFor();
  let seq = 0;
  // Delivered one at a time with a frame between, as the runner streams a seed
  // it is reading off disk — not as one chunk, which is all the other tests do.
  for (let n = 0; n < 30; n++) {
    await say(
      page,
      { kind: 'text', text: `Message ${n}: ` + 'something said at some length. '.repeat(6) },
      ++seq,
    );
  }
  await say(page, { kind: 'text', text: 'THE NEWEST THING SAID' }, ++seq);
  const gap = await page.evaluate(() => {
    const box = document.querySelector('.transcript');
    return box ? box.scrollHeight - box.scrollTop - box.clientHeight : -1;
  });
  expect(gap).toBeLessThan(4);
});

test('a stream that stays dead says so, and says it once @ phone width', async ({
  page,
}, testInfo) => {
  // The only connection state the reader was ever shown was the roster's.
  // That poll covers the Mac being unreachable; it says nothing when the stream
  // alone dies, and then the transcript simply stops, which on screen is a
  // session that is thinking. This is that case.
  //
  // The clock is driven, not waited out. The marker is deliberately patient
  // — the browser retries on its own about every three seconds — so a real wait
  // would put eight seconds into this suite to watch a timer.
  await page.clock.install();
  await mockRunner(page);
  // Registered after `mockRunner`'s, so this one wins: playwright matches the
  // last route first.
  await page.route('**/api/sessions/*/events', (r) => r.abort());
  await page.goto(`/s/${RUNNING.id}`);

  const marker = page.getByText('Not live.');
  await expect(marker, 'warned before the browser had a chance to retry').toBeHidden();
  await page.clock.fastForward('00:10');
  await expect(marker).toBeVisible();
  await expect(page.getByText('not below yet')).toBeVisible();
  // Once, not twice: the roster's poll is answering here, which is the whole
  // point — the app looks healthy and only this transcript is not. Case-insensitive
  // on purpose — an exact string here would go quietly vacuous the next time the
  // sentence is recapitalised.
  await expect(page.getByText(/cannot reach the runner/i)).toBeHidden();
  expect(await marker.count(), 'the marker was drawn more than once').toBe(1);

  await expectCleanLayout(page, testInfo);
});

test('a message sent mid-answer does not cut the answer in half @ phone width', async ({
  page,
}, testInfo) => {
  // The defect, and the visible half of it was the lesser half. A message
  // sent while the model is typing is shown at once, so the next delta found it
  // at the end of the transcript and started a NEW block. A paragraph break
  // mid-sentence is what you notice; a fenced code block cut across two renders
  // is what actually breaks, because each half is then invalid markdown and
  // neither renders as code.
  //
  // The CLI parks a message sent mid-turn and reads it when the turn ends, so it
  // interrupted nothing: the Mac holds it (`Summary.messages`), it is shown after
  // the conversation, and the answer stays one block.
  await mockRunner(page);
  await page.route('**/api/state', (r) =>
    r.fulfill({
      json: {
        ...STATE,
        sessions: [
          {
            ...RUNNING,
            messages: [
              { id: 'm-1', text: 'sent while you were typing', at: Date.now(), stands: 'unread' },
            ],
          },
        ],
      } satisfies Overview,
    }),
  );
  await routeStream(page, (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: (
        [
          { kind: 'text', text: 'Here it is:\n\n```ts\nconst a = 1;\n' },
          { kind: 'text', text: 'const b = 2;\n```\n' },
        ] satisfies Timed[]
      )
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.goto(`/s/${RUNNING.id}`);
  await expect(page.getByText('sent while you were typing')).toBeVisible();

  // One code block holding BOTH lines — not two, and not a stray ``` on screen.
  const code = page.locator('.body pre');
  await expect(code).toHaveCount(1);
  await expect(code).toContainText('const a = 1;');
  await expect(code).toContainText('const b = 2;');
  // The answer's own block, not every `.body` on the page — yours is one too.
  await expect(page.locator('.body').first(), 'a fence survived as literal text').not.toContainText(
    '```',
  );

  await expectCleanLayout(page, testInfo);
});
