import {
  BUSY_BAR,
  ENDED,
  NAMED,
  RUNNING,
  STATE,
  TASKS,
  expect,
  expectBackClosesOverlay,
  expectCleanLayout,
  expectIconsCentred,
  expectNoPinnedOverlap,
  expectOneLine,
  expectThumbTargets,
  expectUpInTheBar,
  leadingGlyph,
  mockRunner,
  openTools,
  test,
  withWakeLock,
  type Commands,
  type Conversation,
  type Described,
  type Overview,
  type Refusal,
} from './console';

test.use({ serviceWorkers: 'block' });

test('the toolbar says which session this is, beside what can be done to it @ phone width', async ({
  page,
}) => {
  await mockRunner(page);
  await page.route('**/api/state', (r) => r.fulfill({ json: NAMED }));
  await page.goto(`/s/${RUNNING.id}`);
  await expect(page.locator('ui-scaffold h1')).toHaveText('health');
  // The ⋮ beside it is a glyph in a circle, and it has to be in the middle of it.
  await expectIconsCentred(page);
  // Named by what it is called rather than by a class, so the assertion is about
  // what a person — or a screen reader — can find.
  await expect(
    page.locator('ui-scaffold').getByRole('button', { name: /what to do with/ }),
  ).toHaveText('more_vert');
  // The path is what the name replaced, and the session's own header no longer
  // prints it: it is in the sheet behind the name.
  //
  // Scoped to the header rather than to the page. A transcript is FULL of
  // paths — every `Read` argument is one — so asserting the path is nowhere on
  // screen fails against a page doing exactly what it should.
  await expect(page.locator('.head')).not.toContainText('/home/example/Code');
});

test('the bar is a session bar before the runner has answered @ phone width', async ({ page }) => {
  // A cold launch lands inside a conversation — the wrapper reopens on the
  // page it remembers — so the toolbar decides by route, not by whether
  // `/api/state` has come back, or it flashes the list's bar first.
  //
  // The reply is held rather than raced: the gap is one request long, which is
  // milliseconds on loopback and luck in a test.
  await mockRunner(page);
  let answer: (() => void) | undefined;
  await page.route('**/api/state', async (route) => {
    if (answer) {
      await route.fulfill({ json: NAMED });
      return;
    }
    await new Promise<void>((go) => (answer = go));
    await route.fulfill({ json: NAMED });
  });
  await page.goto(`/s/${RUNNING.id}`);

  // Nothing has answered yet, and the way out is already on screen.
  await expect(
    page.locator('ui-scaffold [aria-label="all sessions"]'),
    'the bar is still the list’s',
  ).toBeVisible();
  await expect(
    page.locator('ui-scaffold h1', { hasText: /^console$/ }),
    'the root headline flashed up',
  ).toHaveCount(0);

  answer?.();
  await expect(page.locator('ui-scaffold h1')).toHaveText('health');
});

test('a session with no name yet says where it runs, and which one it is @ phone width', async ({
  page,
}) => {
  // The state every session starts in: the runner has not read a name out of the
  // transcript, and the bar still has to say which conversation this is.
  //
  // The folder is not enough on its own any more. Every session is started
  // in `~/Code`, the parent of every repository, so the folder answers the same
  // for all of them — the short id is what tells two new sessions apart, and it
  // is also what claims a task list.
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  await expect(page.locator('ui-scaffold h1')).toHaveText('decode · 6f7c2f11');
  await expect(page.locator('ui-scaffold h1')).toHaveClass(/provisional/);
});

test('the screen can be kept on from either screen @ phone width', async ({ page }) => {
  // Watching a session work is done with no hands for minutes at a time, and the
  // phone's display timeout cannot tell looking from idling. The choice belongs
  // wherever somebody is when they make it — the session they are about to
  // watch, or the list they are about to pick one from.
  await withWakeLock(page);
  await mockRunner(page);

  for (const url of ['/', `/s/${RUNNING.id}`]) {
    await page.goto(url);
    const button = page.locator('ui-scaffold .awake');
    await button.waitFor();
    // Hollow until it is holding anything, and the label says what pressing it
    // will do rather than what the state is.
    await expect(button).toHaveAttribute('aria-pressed', 'false');
    await expect(button.locator('mat-icon')).toHaveText('bedtime');
    await expect(button).toHaveAttribute('aria-label', 'keep the screen on');

    await button.click();
    await expect(button).toHaveAttribute('aria-pressed', 'true');
    await expect(button.locator('mat-icon')).toHaveText('bedtime_off');
    await expect(button).toHaveAttribute('aria-label', 'let the screen sleep');

    await button.click();
    await expect(button).toHaveAttribute('aria-pressed', 'false');
  }
});

test('a browser that cannot keep the screen on is not offered it @ phone width', async ({
  page,
}) => {
  // 412px of toolbar is already a back arrow, a name that has to be allowed to
  // lose, and a ⋮. A control that cannot do anything is the one thing there that
  // need not be — and a disabled button says "not now" where the truth is
  // "never here".
  await page.addInitScript(() => {
    // Off the PROTOTYPE, not off the instance. `wakeLock` is an accessor on
    // `Navigator.prototype`, so `delete navigator.wakeLock` removes an own
    // property that was never there and silently succeeds — the first version of
    // this test did exactly that and asserted against a browser that still had
    // the API. `delete` rather than assigning undefined, because the app asks
    // with `in`, which an own property set to undefined satisfies.
    delete (Navigator.prototype as { wakeLock?: unknown }).wakeLock;
  });
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('ui-scaffold h1').waitFor();
  await expect(page.locator('ui-scaffold .awake')).toHaveCount(0);
});

test('the toolbar starts in the same place on both screens @ phone width', async ({ page }) => {
  // Entering a session and leaving one swaps the leading glyph — the terminal
  // mark for a back arrow — and the eye tracks a mark that stays put.
  //
  // They do not line up by default, and the reason is invisible in the
  // markup. An icon button carries 8px of padding inside its own box and a
  // bare `mat-icon` carries none, so the arrow begins 8px further in than the
  // mark it replaces. Nothing overflows, nothing is clipped, and no check that
  // measures one screen at a time can see it: the fault is a difference BETWEEN
  // two renders, which is why this test loads both.
  await mockRunner(page);
  await page.goto('/');
  // `.first()`, because the bar now carries a second glyph: the screen-awake
  // control sits at the end of both branches, and a bare locator matching two
  // elements is a strict-mode violation rather than a measurement.
  await page.locator('ui-scaffold mat-icon').first().waitFor();
  const list = await leadingGlyph(page);
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('ui-scaffold [aria-label="all sessions"]').waitFor();
  const session = await leadingGlyph(page);
  expect(list, 'the list has no leading glyph to measure').toBeGreaterThan(0);
  expect(
    Math.abs(session - list),
    `the leading glyph jumps ${Math.abs(session - list)}px on entering a session`,
  ).toBeLessThanOrEqual(1);
});

test('a name too long for the bar gives way rather than pushing @ phone width', async ({
  page,
}, testInfo) => {
  // A name is arbitrary text from a transcript, and the toolbar is a fixed
  // row holding two other things that must stay reachable. Being cut off is the
  // intended outcome; the failures are the ways it declines to be.
  //
  // Page-level checks cannot see this: a label can spill out of its box and
  // paint over a neighbour with nothing off the screen. And a block's rect is its
  // box, which does not grow with text spilling out of it, so box measurements
  // stay true while the glyphs paint over the ⋮. Hence the assertions are about
  // the label, and the clipping is asserted directly.
  await mockRunner(page);
  await page.route('**/api/state', (r) =>
    r.fulfill({
      json: {
        ...STATE,
        sessions: [{ ...RUNNING, name: 'health-sync-backend-decode-matcher-gate-quantiser' }],
      } satisfies Overview,
    }),
  );
  await page.goto(`/s/${RUNNING.id}`);
  // The name, not the heading: the bar draws its heading before the name arrives.
  await expect(page.locator('ui-scaffold h1')).toHaveText(
    'health-sync-backend-decode-matcher-gate-quantiser',
  );
  await expectCleanLayout(page, testInfo, { allow: BUSY_BAR });
  await expectThumbTargets(page);

  const bar = await page.evaluate(() => {
    // `HTMLElement`, not `Element`: `scrollWidth`/`clientWidth` are what this
    // measures, and they are the whole assertion.
    const name = document.querySelector<HTMLElement>('ui-scaffold h1')!;
    const menu = document
      .querySelector('ui-scaffold button[aria-haspopup="menu"]')!
      .getBoundingClientRect();
    const leave = document
      .querySelector('ui-scaffold [aria-label="all sessions"]')!
      .getBoundingClientRect();
    return {
      wanted: name.scrollWidth,
      given: name.clientWidth,
      clips: getComputedStyle(name).overflowX,
      label: name.getBoundingClientRect(),
      menuLeft: menu.left,
      menuRight: menu.right,
      leaveRight: leave.right,
      page: window.innerWidth,
    };
  });
  // It really was cut: the text wants more room than the box it was given, which
  // is the state an ellipsis is drawn in. `clientWidth` is 0 for an inline
  // element, so this also fails if the span ever stops being a box.
  expect(bar.given, 'the name is not a box, so nothing can clip it').toBeGreaterThan(0);
  expect(bar.wanted, 'nothing was truncated — the fixture name is too short').toBeGreaterThan(
    bar.given,
  );
  // And the box it was given is one that cuts, rather than one the text runs out
  // of. The computed value, not the rule as written: this is what the cascade
  // arrived at after Material's own styles had their say, and it is the only
  // evidence available — nothing a rect reports distinguishes cut from spilling,
  // per the note above. Together with the two lines before it, this says the
  // text is longer than its box AND cannot be painted outside it.
  expect(bar.clips, 'the name does not clip, so its ellipsis is decoration').toBe('hidden');
  // And the box gave way to both of its neighbours rather than pushing either.
  expect(bar.label.left, 'the name is painting over the back button').toBeGreaterThanOrEqual(
    bar.leaveRight,
  );
  expect(bar.label.right, 'the name is under the overflow button').toBeLessThanOrEqual(
    bar.menuLeft,
  );
  // The overflow button keeps its place at the end of the row.
  expect(bar.menuRight, 'the overflow button was pushed off the edge').toBeLessThanOrEqual(
    bar.page,
  );
});

test('what the session may do without asking is on the header @ phone width', async ({
  page,
}, testInfo) => {
  // The card and the header draw the glyph only for a session whose mode the
  // runner has read, so a fixture with no mode measures a row missing an element.
  // A button here inherits the app-wide 3rem floor from `styles.scss`, which the
  // header's height must not.
  //
  // Asserted by presence and by box rather than by text: the neighbouring
  // `.facts` checks all use `toContain`, so an extra ligature slips past them
  // and so does its disappearance.
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  const mode = page.locator('.head .mode');
  await expect(mode).toHaveCount(1);
  // The name is what the icon means, and it is the only thing that says so on
  // this row — the label is one tap away in the menu that also teaches it.
  await expect(mode).toHaveAttribute('aria-label', /edits/i);

  // In the row, not overhanging it. The regression was exactly this: a
  // glyph whose box grew past the line it sits on, pushing the header's height
  // and shoving the model name right.
  // Against the text beside it, NOT against its own row. The first
  // version of this compared the glyph to `.facts` and passed while the defect
  // was reintroduced on purpose — the row grows to fit whatever is in it, so
  // that comparison can never fail. Measured both ways at 412px:
  //
  //           head    facts   icon    model text
  //   clean   19.19   19.19   17.59   16
  //   broken  48      48      48      16
  //
  // The row and the icon move together and say nothing; the ratio to the text
  // is 1.1 against 3.0. That is the rule `sessions-view.scss` already states for
  // the card's tally — the glyph "has to be the number's height, not the row's".
  const box = await page.evaluate(() => {
    const h = (sel: string) => document.querySelector(sel)!.getBoundingClientRect().height;
    return { icon: h('.head .mode'), text: h('.head .model') };
  });
  expect(
    box.icon / box.text,
    `the glyph is ${box.icon}px beside ${box.text}px of text, so it is sized to the row`,
  ).toBeLessThan(1.5);

  await expectOneLine(page, '.session .head');
  await expectCleanLayout(page, testInfo);
  await expectThumbTargets(page);
});

test('the session is still named after scrolling to the end @ phone width', async ({ page }) => {
  // Which conversation you are in is the one fact worth having on screen at all
  // times — the console drives a dozen at once and they differ by name. Both the
  // toolbar and the facts row sit outside the scrolling region (`.transcript` is
  // the only thing on this page that scrolls), so this measures that they do.
  await mockRunner(page);
  await page.route('**/api/state', (r) => r.fulfill({ json: NAMED }));
  await page.goto(`/s/${RUNNING.id}`);
  // Opened, because this needs a transcript tall enough to scroll and a folded
  // run is deliberately short.
  await openTools(page);
  await page.getByText('verified_cli').first().waitFor();
  const tops = () =>
    page.evaluate(() => ({
      name: document.querySelector('ui-scaffold h1')!.getBoundingClientRect().top,
      facts: document.querySelector('.head .facts')!.getBoundingClientRect().top,
    }));
  const before = await tops();
  await page.evaluate(() => {
    const box = document.querySelector('.transcript')!;
    box.scrollTop = box.scrollHeight;
  });
  await page.evaluate(
    () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))),
  );
  const scrolled = await page.evaluate(() => document.querySelector('.transcript')!.scrollTop);
  expect(scrolled, 'the transcript did not move, so this proves nothing').toBeGreaterThan(100);
  expect(await tops(), 'the session scrolled away with its conversation').toEqual(before);
  await expect(page.locator('ui-scaffold h1')).toBeInViewport();
});

test('the details sheet holds what the page has no room for @ phone width', async ({
  page,
}, testInfo) => {
  // The first overlay this suite has ever measured. The console has had a
  // menu in the toolbar since it was written and no check has ever opened it: an
  // overlay renders outside the component tree, so nothing on the page below is
  // evidence about it. A sheet of paths and identifiers is exactly the content
  // that pushes a 412px screen sideways.
  await mockRunner(page);
  await page.route('**/api/state', (r) => r.fulfill({ json: NAMED }));
  await page.goto(`/s/${RUNNING.id}`);
  // Behind the ⋮, where everything else you can do to a session already was.
  await page
    .locator('ui-scaffold')
    .getByRole('button', { name: /what to do with/ })
    .click();
  await page.getByRole('menuitem', { name: 'Details' }).click();
  const sheet = page.locator('.session-sheet');
  await sheet.waitFor();

  const said = await sheet.innerText();
  // The path, which nothing on the screen shows any more.
  expect(said).toContain(RUNNING.dir);
  // The session id, which nothing in the console showed anywhere.
  expect(said).toContain(RUNNING.id);
  // The model as it is shipped, not the one word the facts row has room for.
  expect(said).toContain('claude-opus-5[1m]');
  // The permission mode in words. The facts row has only its icon, and a
  // `title=` tooltip on a phone is text nobody can reach.
  expect(said).toContain('Bypass Permissions');
  // How much has been said, which the list card cannot carry: four facts in
  // that row wrap it. Beside how full the context is, because the
  // gap between the two is the fact neither one states.
  expect(said).toContain('62 MB');
  expect(said).toContain('640k / 1M');
  // Whole, and attributed. The card clamps this sentence to two lines, so
  // being cut there is a reason to open the sheet — finding it cut here as well
  // would be the panel failing at its one job. And it is the only line in it a
  // model wrote, which the sheet has room to say in words.
  expect(said).toContain('running the golden set to see which journeys moved');
  expect(said).toContain('written by Haiku');
  // The other name, and the sentence saying the two disagree. Nothing else in
  // the console shows what another session would have to type to reach this one.
  // Lowercased: the sheet's labels are uppercased in CSS, which `innerText`
  // carries through.
  expect(said.toLowerCase()).toContain('known to peers as');
  expect(said).toContain('code-a7');
  expect(said, 'the disagreement is drawn but not explained').toContain('next resume');

  await expectCleanLayout(page, testInfo, { root: '.session-sheet' });
});

test('the task sheet opens on what is left rather than what is done @ phone width', async ({
  page,
}, testInfo) => {
  // The list is mostly finished work, by an order of magnitude. One live
  // session here keeps 355 tasks: 307 done, 38 open, 10 underway. Opening on all
  // of them puts three hundred completed rows above the eight that matter, so
  // the default is what is left — and the toggle is there because the finished
  // ones are a written record worth reading, not because the list is a total.
  await mockRunner(page);
  await page.route('**/api/sessions/*/tasks', (r) => r.fulfill({ json: TASKS }));
  // Markdown, with a code block in it. These are written up as reports —
  // headings, bold, fenced commands — and the transcript's own renderer went 618px
  // past the right edge the first time a fixture had code in it. This sheet gets
  // the same rules, so it gets the same fixture.
  await page.route('**/api/sessions/*/tasks/*', (r) =>
    r.fulfill({
      json: {
        description: [
          '## What moved',
          '',
          '**Measured both ways**, and the second arm moved.',
          '',
          '```',
          'nix develop --command cargo test --workspace -- --exact the_matcher_gate_is_bit_exact',
          '```',
          '',
          '- [x] ported',
          '- [ ] blessed',
        ].join('\n'),
      } satisfies Described,
    }),
  );
  await page.goto(`/s/${RUNNING.id}`);
  await page
    .locator('ui-scaffold')
    .getByRole('button', { name: /what to do with/ })
    .click();
  // Open over total on the menu item itself, so "is there anything left?" is
  // answered without opening the sheet at all. Off the poll rather than counted
  // on the tap: the runner sweeps every session's list, so this label is right
  // the moment the menu opens and the front page shows the same numbers.
  await expect(page.locator('.session-menu .tally')).toHaveText('2/3');
  await page.getByRole('menuitem', { name: 'Tasks' }).click();
  const sheet = page.locator('.session-sheet');
  await sheet.waitFor();

  // Underway first, then open. What is done is not on screen at all.
  await expect(sheet.locator('.subject')).toHaveText([
    'port the matcher gate',
    'refresh the recovery bundle before the next machine',
    'write the rule up',
    'tidy the fixture names',
    'reconcile the overnight decoder against the joint model and say which of the two owns journey reconstruction',
    'was waiting on the effect language',
  ]);
  // With the numbers the session itself uses, in the same order — a session
  // writes `#N done` in its own prose, so the row has to be findable by it.
  //
  // And in the service's order within a status, not one of ours. Task 7 is
  // ranked above the unranked task 100 and task 99 below it, and that is how
  // they arrive — nothing here re-sorts on the rank, or there would be two
  // orderings to keep true and they would disagree the first time either moved.
  await expect(sheet.locator('.num')).toHaveText(['101', '7', '100', '99', '412', '413']);

  // A deadline is drawn, and it does NOT reorder anything. Task 412 is overdue
  // and sits where the service put it — last — because a deadline is evidence
  // for a rank rather than a competing answer to what-next. The service has a
  // test that fails if anyone makes it sort; this is the same rule on the phone.
  await expect(sheet.locator('.when')).toHaveCount(2);
  await expect(sheet.locator('.when.late')).toHaveCount(1);
  await expect(sheet.locator('.when.late')).toHaveAttribute(
    'aria-label',
    'overdue — was due 2026-08-01',
  );
  // Not red merely for having one: a date that has not been missed is a fact
  // about the task, not a problem with it.
  await expect(sheet.locator('.when:not(.late)')).toHaveAttribute('aria-label', 'due 2026-09-01');

  // One block mark, not two. Task 413 still carries `blocked_on` and the
  // service says it is no longer blocked — the link is kept after a blocker
  // closes, as a record of how the work went. A row deciding for itself from the
  // ids would say the opposite, and this client has no way to know better.
  await expect(sheet.locator('.held')).toHaveCount(1);
  await expect(sheet.locator('.held')).toHaveAttribute('aria-label', 'waiting on #92, #93');

  // The rank is drawn, and only where there is one. Two chips on four
  // rows: the other two are unranked, which is what almost every task is, and a
  // placeholder on those would be the mark most of this list wore.
  await expect(sheet.locator('.rank')).toHaveText(['P0', 'P3']);
  // Loud only for the rank that outranks the untriaged. P3 is below it, so its
  // chip is the quiet one: making "when there is room" the brightest thing on
  // the screen is the defect, not the feature.
  await expect(sheet.locator('.rank.above')).toHaveText(['P0']);

  // The write-up is behind a tap, because sending it with the list would be a
  // megabyte and a half to draw two subjects.
  await expect(sheet.locator('.said')).toHaveCount(0);
  await sheet.getByRole('button', { name: /port the matcher gate/ }).click();
  await expect(sheet.locator('.said')).toContainText('the second arm moved');
  // Rendered rather than shown as characters: a heading is an element, the
  // command is a code block, and the checkbox states are distinguishable — all
  // three read as punctuation before the pipe was wired in.
  await expect(sheet.locator('.said h2')).toHaveText('What moved');
  await expect(sheet.locator('.said strong')).toHaveText('Measured both ways');
  await expect(sheet.locator('.said pre')).toContainText('cargo test --workspace');
  await expect(sheet.locator('.said li.task').first()).toContainText('☑');

  // And the toggle brings back what is closed, saying how much it is. Both kinds
  // of closed: "1 done" would have offered to reveal one row and revealed two.
  await sheet.getByRole('radio', { name: /All \(2 closed\)/ }).click();
  // Eight — the six open ones and the two that were closed. This asked for
  // six, which is the count BEFORE the click, and passed for two months because
  // `toHaveCount` polls: the first poll landed before change detection had
  // appended the closed rows, matched the stale DOM and returned. Under load the
  // first poll arrives after the render instead, sees the truth and fails — read
  // as "the sheet opened on All" when the sheet has done nothing wrong and the
  // assertion is measuring its own race.
  await expect(sheet.locator('.subject')).toHaveCount(8);
  // Finished before abandoned, and the dropped row says which it is rather than
  // borrowing the tick. Read off the mark's label, because that is also what a
  // screen reader gets.
  await expect(sheet.locator('.mark').first()).toHaveAttribute('aria-label', 'underway');
  // By class, not by position: `.task` also matches the checkbox items inside
  // the write-up opened above, and this list has one of those on screen.
  await expect(sheet.locator('.task.dropped .mark')).toHaveAttribute('aria-label', 'dropped');
  await expect(sheet.locator('.tasks > .task').last()).toHaveClass(/dropped/);
  await expect(sheet.locator('.task.dropped .subject')).toHaveText(
    'move the per-session repo claim into the service',
  );

  await expectCleanLayout(page, testInfo, { root: '.session-sheet' });
  await expectThumbTargets(page);
});

test('back dismisses an overlay rather than the page under it @ phone width', async ({ page }) => {
  // On the phone this is a gesture, not a button, and it is the one people make
  // without looking. An overlay takes no part in history by itself, so the
  // question is what a back press does while one is open — dismiss it, or walk
  // out of the session with the sheet still on top of whatever it lands on.
  await mockRunner(page);
  await page.route('**/api/state', (r) => r.fulfill({ json: NAMED }));
  // Walked in from the list rather than deep-linked, so there is real history
  // behind this page: `goto` alone leaves one entry and back exits to about:blank,
  // which measures Playwright rather than the console.
  const at = `/s/${RUNNING.id}`;
  await page.goto('/');
  await page.locator('.session').first().click();
  await page.locator('.transcript').waitFor();
  await page
    .locator('ui-scaffold')
    .getByRole('button', { name: /what to do with/ })
    .click();
  await page.getByRole('menuitem', { name: 'Details' }).click();
  await page.locator('.session-sheet').waitFor();

  await page.goBack();
  await expect(page.locator('.session-sheet'), 'the sheet outlived the page').toHaveCount(0);
  expect(new URL(page.url()).pathname, 'back left the session as well as the sheet').toBe(at);
  // And the step it spent was the sheet's own, not the session's: one more back
  // is what leaves.
  await page.goBack();
  expect(new URL(page.url()).pathname, 'the session would not let go').toBe('/');
});

test('back closes the start sheet rather than the app @ phone width', async ({ page }) => {
  // The list is the root, so there is nothing behind it: a back press with this
  // open and no history entry of its own goes out of the console altogether. In
  // a browser that is a lost page; in the WebView wrapper it is the app closing.
  await mockRunner(page);
  await page.goto('/');
  await page.locator('.page .add').click();
  await page.locator('.start-sheet').waitFor();

  await page.goBack();
  await expect(page.locator('.start-sheet'), 'the sheet outlived the gesture').toHaveCount(0);
  expect(new URL(page.url()).pathname, 'back left the console, not the sheet').toBe('/');
});

test('a sheet put away by hand leaves no step behind @ phone width', async ({ page }) => {
  // The other half of giving an overlay a history entry, and the half that
  // is easy to leave out: a sheet closed the ordinary way — a tap on the
  // backdrop — has to take its entry with it. Otherwise the step outlives the
  // panel it stood for, and the next back press is spent on nothing at all. A
  // phone that ignores a gesture reads as a phone that has frozen.
  await mockRunner(page);
  await page.route('**/api/state', (r) => r.fulfill({ json: NAMED }));
  await page.goto('/');
  await page.locator('.session').first().click();
  await page.locator('.transcript').waitFor();
  await page
    .locator('ui-scaffold')
    .getByRole('button', { name: /what to do with/ })
    .click();
  await page.getByRole('menuitem', { name: 'Details' }).click();
  await page.locator('.session-sheet').waitFor();

  // Away by hand, not by back: the backdrop is what a thumb reaches first.
  // `.last()` is the sheet's own — the menu that opened it leaves its backdrop in
  // the DOM behind this one while it fades.
  //
  // Aimed at the top of the screen rather than clicked at its centre. The
  // backdrop covers the viewport, so its centre is *under the sheet* once the
  // sheet is tall enough — and it grew when the history and the fullness moved
  // into it. Playwright then waits ninety seconds for a point the sheet is
  // sitting on. The strip of backdrop above a bottom sheet is where a thumb
  // aims anyway.
  await page
    .locator('.cdk-overlay-backdrop')
    .last()
    .click({ position: { x: 8, y: 8 } });
  await expect(page.locator('.session-sheet')).toHaveCount(0);

  await page.goBack();
  expect(new URL(page.url()).pathname, 'the first back press was spent on nothing').toBe('/');
});

test('below the list, the bar leads up, and back closes a sheet and only the sheet @ phone width', async ({
  page,
}) => {
  // The scaffold's own checks (`@xinutec/ui-harness`), run against this app's
  // routes: whether a screen has a parent is declared in `app.routes.ts`, and
  // whether a sheet is wired into history is decided where it is opened.
  await mockRunner(page);
  await page.route('**/api/state', (r) => r.fulfill({ json: NAMED }));
  await page.goto(`/s/${RUNNING.id}`);
  await expect(page.locator('ui-scaffold h1')).toHaveText('health');
  await expectUpInTheBar(page);
  await expectBackClosesOverlay(page, async () => {
    await page.locator('ui-scaffold button[aria-label^="what to do with"]').click();
    await page.getByRole('menuitem', { name: 'Details' }).click();
  });
  await page.goto('/reader');
  await expectUpInTheBar(page);
});

test('leaving a session leaves its name behind @ phone width', async ({ page }) => {
  // The toolbar kept the session you had just left. `ngOnDestroy` clears
  // the open conversation, and the five-second poll's request does not stop when
  // the page does — so a response already in flight lands after the clear and
  // puts it back. The list then shows a name and a ⋮ for a session nobody is in,
  // and the menu behind that ⋮ acts on it.
  //
  // Held in flight deliberately rather than raced: the defect needs a response
  // that arrives after the route has changed, which is a matter of milliseconds
  // on a phone and of luck in a test.
  await mockRunner(page);
  let answer: (() => void) | undefined;
  await page.route('**/api/state', async (route) => {
    if (answer) {
      // Every later poll answers at once; only the first is held.
      await route.fulfill({ json: NAMED });
      return;
    }
    await new Promise<void>((go) => (answer = go));
    await route.fulfill({ json: NAMED });
  });
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('.transcript').waitFor();
  // Away before the runner has answered.
  await page.locator('ui-scaffold [aria-label="all sessions"]').click();
  await page.locator('.session').first().waitFor();
  answer?.();
  // Long enough for the held response to land and be acted on.
  await page.waitForTimeout(300);
  await expect(
    page.locator('ui-scaffold h1'),
    'the list is titled with the session just left',
  ).toHaveText('console');
  // Not "no buttons" or "no menu": the list has both. What this test is about is
  // a session no longer on screen
  // still being ACTIONABLE, so it names the control that acts on one: the ⋮
  // labels itself with the conversation it belongs to.
  await expect(
    page.locator('ui-scaffold button[aria-label^="what to do with"]'),
    'the ⋮ still acts on the session just left',
  ).toHaveCount(0);
  await expect(page.locator('ui-scaffold [aria-label="all sessions"]')).toHaveCount(0);
});

test('a session that has stopped reading names it, with the cure @ phone width', async ({
  page,
}, testInfo) => {
  // The row this replaces said the ordinary thing. A deaf session's
  // messages carry the same *waiting to be read* marker as a busy session's, so
  // both episodes of it cost a diagnosis by hand. This is the banner
  // that says which it is — see `session::Session::deaf`.
  //
  // At phone width because that is where it has to fit: a sentence, a duration
  // and a button on one row beside a transcript that is already tight.
  await mockRunner(page);
  const deaf = { ...RUNNING, busy: undefined, waiting: 0, unread: 2, deaf: 1284 };
  await page.route('**/api/state', (r) =>
    r.fulfill({ json: { ...STATE, sessions: [deaf, ENDED] } satisfies Overview }),
  );
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('.deaf').waitFor();
  await expect(page.locator('.deaf')).toContainText('Not reading');
  await expect(page.locator('.deaf')).toContainText('21m');
  await expect(page.getByRole('button', { name: 'Restart it' })).toBeEnabled();
  await expectCleanLayout(page, testInfo, { allow: BUSY_BAR });
  await expectNoPinnedOverlap(page);
});

test('an ended session offers the way back @ phone width', async ({ page }, testInfo) => {
  // The list has no way back from an ended session: the roster keeps it, so its
  // card stays a link to this page, and the resume affordance on the front page
  // exists only for a conversation the console is NOT holding. This is it.
  await mockRunner(page);
  await page.goto(`/s/${ENDED.id}`);
  await page.locator('.deaf.ended').waitFor();
  await expect(page.getByRole('button', { name: 'Start it again' })).toBeEnabled();
  await expectCleanLayout(page, testInfo, { allow: BUSY_BAR });
});

test('a working session can be renamed from the menu @ phone width', async ({ page }, testInfo) => {
  // `/rename` cannot do this. A slash command sent to a busy session is
  // parked and released as a prompt, so the model reads the words and the name
  // never changes. This route is a control request, which
  // the CLI answers mid-turn.
  let sent: Record<string, unknown> | undefined;
  await mockRunner(page);
  await page.route('**/api/sessions/*/rename', (r) => {
    sent = r.request().postDataJSON() as Record<string, unknown>;
    return r.fulfill({ json: RUNNING });
  });
  await page.goto(`/s/${RUNNING.id}`);
  await page
    .locator('ui-scaffold')
    .getByRole('button', { name: /what to do with/ })
    .click();
  await page.getByRole('menuitem', { name: 'Rename' }).click();

  const name = page.getByLabel('name');
  await expect(name, 'the sheet did not open').toBeVisible();
  await name.fill('tasks');
  await expectCleanLayout(page, testInfo, { root: 'mat-bottom-sheet-container' });
  await page.getByRole('button', { name: /^rename$/ }).click();

  await expect.poll(() => sent).toMatchObject({ title: 'tasks' });
  // Dismissed rather than redrawn: the new name arrives from the transcript on
  // the next poll, and claiming it from this response would be the console
  // reporting its own intent again.
  await expect(page.locator('mat-bottom-sheet-container')).toHaveCount(0);
});

test('the rename sheet offers the name a model wrote, and does not apply it @ phone width', async ({
  page,
}, testInfo) => {
  // The sheet opened empty for a conversation nobody had named, which is the one
  // it is opened for most — and the card a screen away has had a sentence about
  // that conversation the whole time. This is the second line of the same call.
  let sent: Record<string, unknown> | undefined;
  await mockRunner(page);
  await page.route('**/api/state', (r) =>
    r.fulfill({
      json: {
        ...STATE,
        gists: {
          [RUNNING.id]: {
            text: 'porting the last of the matcher gate to Lean and running the golden set',
            at: 1785600000000,
            name: 'Lean port',
            bytes: 40_000_000,
          },
        },
      } satisfies Overview,
    }),
  );
  await page.route('**/api/sessions/*/rename', (r) => {
    sent = r.request().postDataJSON() as Record<string, unknown>;
    return r.fulfill({ json: RUNNING });
  });
  await page.goto(`/s/${RUNNING.id}`);
  await page
    .locator('ui-scaffold')
    .getByRole('button', { name: /what to do with/ })
    .click();
  await page.getByRole('menuitem', { name: 'Rename' }).click();

  const name = page.getByLabel('name', { exact: true });
  await expect(name, 'the sheet did not open').toBeVisible();
  // The suggestion is beside the field, not in it. A name is stickier than
  // a summary, and a guess sitting in the box is one Enter away from being the
  // conversation's name without anybody having agreed to it.
  await expect(name, 'the suggestion was put in the box').toHaveValue('');
  const offer = page.getByRole('button', { name: /use the suggested name/ });
  await expect(offer, 'nothing was offered').toBeVisible();
  // The action has to survive the extra row. This sheet sits on the bottom
  // of a phone and the offer goes above the button that does the renaming, so a
  // row added here is a row of somewhere else — and the thing pushed off the
  // end would be the one control the sheet exists for.
  await expect(
    page.getByRole('button', { name: /^rename$/ }),
    'the offer pushed the rename button off the screen',
  ).toBeInViewport({ ratio: 1 });
  await expectCleanLayout(page, testInfo, { root: 'mat-bottom-sheet-container' });

  await offer.click();
  await expect(name).toHaveValue('Lean port');
  // Gone once taken: a button that would now do nothing reads as a broken one.
  await expect(offer, 'the offer stayed after it was taken').toHaveCount(0);

  await page.getByRole('button', { name: /^rename$/ }).click();
  await expect.poll(() => sent).toMatchObject({ title: 'Lean port' });
});

test('the rename sheet offers nothing when no model has named the conversation @ phone width', async ({
  page,
}, testInfo) => {
  // The ordinary case for a while yet: every gist on disk predates the second
  // line, and a conversation with no gist has never had one. The sheet must be
  // exactly what it was — an empty box — rather than a gap where an offer goes.
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  await page
    .locator('ui-scaffold')
    .getByRole('button', { name: /what to do with/ })
    .click();
  await page.getByRole('menuitem', { name: 'Rename' }).click();

  await expect(page.getByLabel('name', { exact: true }), 'the sheet did not open').toBeVisible();
  await expect(page.getByRole('button', { name: /use the suggested name/ })).toHaveCount(0);
  await expectCleanLayout(page, testInfo, { root: 'mat-bottom-sheet-container' });
});

test('a session opened with no answer from the Mac reads from the kept copy @ phone width', async ({
  page,
}, testInfo) => {
  // The console is not an offline app and this does not make it one — it
  // has no service worker on purpose, because it sits behind a client-certificate
  // gate. What this covers is narrower: a phone whose tunnel has dropped can
  // still read the session it was
  // looking at. Sending is untouched; a send that cannot reach the Mac keeps its
  // draft in the composer as it always did.
  const id = RUNNING.id;
  // The copy is MADE by reading, not seeded into storage. A fixture
  // written straight into the store pins where the store happens to be today —
  // this walks the path a phone walks: read the conversation once with the
  // tunnel up, then come back to it with the tunnel down.
  await mockRunner(page);
  await page.goto(`/s/${id}`);
  const line = page.getByText('home-manager switch').first();
  await expect(line, 'the conversation was never read in the first place').toBeVisible();
  // The copy is written asynchronously and this has to outlast that. The
  // database opens on the first write, so the copy lands a moment after the
  // conversation appears — a person reading one is there far longer than a test,
  // and navigating immediately races a write that nothing real races. A duration
  // rather than a condition because the only observable is RxDB's own Dexie
  // tables, and a test reading those is pinned to the library's internals rather
  // than to the behaviour.
  await page.waitForTimeout(2000);

  // Now the Mac does not answer: the transcript stream never opens, which is
  // exactly what a dropped tunnel looks like from here.
  await page.route('**/api/sessions/*/events*', (r) => r.abort());
  await page.goto(`/s/${id}`);

  await expect(line, 'the kept copy was not read').toBeVisible();
  // And it says what it is. A transcript that has stopped growing looks
  // exactly like a quiet one, so a copy drawn as though it were the conversation
  // would claim a state that is not true.
  const banner = page.locator('.adrift');
  await expect(banner, 'a copy was drawn as though it were live').toContainText('Kept copy');
  await expectCleanLayout(page, testInfo, { root: '.adrift' });
});

test('a session read before shows its kept copy at once while the Mac is slow @ phone width', async ({
  page,
}, testInfo) => {
  // The common case, not the failed one: a phone that has just woken reaches the
  // Mac seconds late. Until this, nothing was drawn until the stream answered.
  const id = RUNNING.id;
  await mockRunner(page);
  await page.goto(`/s/${id}`);
  const line = page.getByText('home-manager switch').first();
  await expect(line, 'the conversation was never read in the first place').toBeVisible();
  // The copy is written asynchronously; see the test above.
  await page.waitForTimeout(2000);

  // Slow, not dead: the stream answers after four seconds.
  let release: () => void = () => undefined;
  const answered = new Promise<void>((resolve) => (release = resolve));
  await page.route('**/api/sessions/*/events*', async (r) => {
    await answered;
    await r.fallback();
  });
  await page.goto(`/s/${id}`);

  await expect(line, 'the kept copy was not drawn while the Mac was slow').toBeVisible({
    timeout: 1500,
  });
  await expect(page.locator('.catching-up'), 'nothing said it is still arriving').toBeVisible();
  await expect(page.locator('.adrift'), 'slow was drawn as failed').toHaveCount(0);
  await expectCleanLayout(page, testInfo, { allow: BUSY_BAR });

  release();
  await expect(page.locator('.catching-up'), 'the copy outlived the answer').toHaveCount(0);
  await expect(line).toBeVisible();
});

test('session strip — a background call is named, not counted @ phone width', async ({
  page,
}, testInfo) => {
  // *1 background task running* cannot say which; one task with a name is
  // actionable where a number is only a reason to ask.
  //
  // Both labels here are real shapes: `Monitor` carries a written description,
  // and a `Bash` one-liner arrives long enough to overflow a 412px line — which
  // is why the runner cuts to 60 and the strip ellipsises on top of that.
  await mockRunner(page);
  const id = RUNNING.id;
  await page.route('**/api/state', (r) =>
    r.fulfill({
      json: {
        ...STATE,
        sessions: [
          {
            ...RUNNING,
            id,
            background: 2,
            running: [
              { tool: 'Monitor', label: 'HDD→SSD migration progress', task: 'bko3hqzmv' },
              {
                tool: 'Bash',
                label: 'rsync -aHAX --numeric-ids --delete --exclude=.Spotli…',
                task: 'b1nhifqhm',
              },
            ],
          },
          ...STATE.sessions.slice(1),
        ],
      } satisfies Overview,
    }),
  );
  await page.goto(`/s/${id}`);

  // The tool and its label, rather than a tally.
  const strip = page.locator('.update.running');
  await expect(strip).toHaveCount(2);
  await expect(strip.first()).toContainText('Monitor');
  await expect(strip.first()).toContainText('HDD→SSD migration');
  await expect(strip.nth(1)).toContainText('Bash');
  // And the count alone is gone when names are available — both would be the
  // same fact said twice, and the number is the half that could not be acted on.
  // Asserted as the ABSENCE OF THE FALLBACK ROW rather than as text `.update`
  // does not contain: `.update` resolves to two elements here, and a text
  // assertion against a multi-element locator is a strict-mode violation that
  // times out instead of failing on what it was actually asked.
  await expect(page.locator('.update:not(.running)')).toHaveCount(0);

  await expectCleanLayout(page, testInfo);
});

test('a session that has ended dates its background work @ phone width', async ({
  page,
}, testInfo) => {
  // The count is FROZEN once the process is gone, and it was drawn live.
  // `Running::Ended` comes from a task-notification, which only the `claude`
  // process writes — so after it exits nothing can decrement the number, and
  // the card would read `ended · 2 background tasks` for ever.
  //
  // Both surfaces, one test, because the wording has to agree across them: the
  // row is where the contradiction was noticed and the strip is where somebody
  // goes to find out what the work WAS.
  await mockRunner(page);
  const ended = ENDED;
  await page.route('**/api/state', (r) =>
    r.fulfill({
      json: {
        ...STATE,
        sessions: [
          RUNNING,
          {
            ...ended,
            background: 2,
            running: [
              { tool: 'Monitor', label: 'HDD→SSD migration progress', task: 'bko3hqzmv' },
              { tool: 'Bash', label: 'nix build .#console', task: 'b1nhifqhm' },
            ],
          },
        ],
      } satisfies Overview,
    }),
  );
  await page.route('**/api/past', (r) => r.fulfill({ json: [] satisfies Conversation[] }));
  await page.goto('/');

  const row = page.locator('.session', { hasText: 'check the corpus' }).locator('.head');
  await expect(row).toContainText('ended');
  await expect(row).toContainText('2 background tasks when it ended');

  // And the live session beside it is UNCHANGED — the past tense is keyed
  // to `alive`, so a bug that dated everything would still satisfy the row
  // above. The first fixture session is alive and carries no background work,
  // which is also the assertion that `0 background tasks` is still never said.
  await expect(page.locator('.session').first()).not.toContainText('background');

  await expectOneLine(page, '.session .head');
  await expectCleanLayout(page, testInfo);

  // The strip on the page itself: the names stay — work orphaned by a session
  // dying is the case worth a look at `ps` — but a caption dates them.
  await page.goto(`/s/${ended.id}`);
  await expect(page.locator('.update.running')).toHaveCount(2);
  await expect(page.locator('.update:not(.running)')).toContainText(
    'running when the session ended',
  );
  await expectCleanLayout(page, testInfo);

  // The nameless fallback is a SECOND sentence and was the one that read
  // worst — `2 background tasks running` with no names, on a session that had
  // stopped. Reached by a runner too old to send `running`, which is exactly
  // the console this bug outlived.
  await page.route('**/api/state', (r) =>
    r.fulfill({
      json: { ...STATE, sessions: [RUNNING, { ...ended, background: 2 }] } satisfies Overview,
    }),
  );
  await page.reload();
  // The positive assertion FIRST, and the `toHaveCount(0)` only after it.
  // A page mid-reload has no `.update.running` either, so that count passes on
  // its first poll against a DOM that has not rendered yet and asserts nothing
  // — a test measuring its own race.
  // Waiting for the one element that must exist is what makes the absence mean
  // something. Counted at all because `.update` is also the newer-build notice,
  // and a text assertion against two elements is a strict-mode violation.
  await expect(page.locator('.update')).toHaveCount(1);
  await expect(page.locator('.update')).toContainText(
    '2 background tasks running when the session ended',
  );
  await expect(page.locator('.update.running')).toHaveCount(0);
  await expectCleanLayout(page, testInfo);
});

test('a slash command completes from what the session can be told @ phone width', async ({
  page,
}, testInfo) => {
  // The commands and their values come from the session (`initialize`), not a
  // list in the app: this one offers effort's levels because its hint lists them.
  await mockRunner(page);
  await page.route('**/api/sessions/*/commands', (r) =>
    r.fulfill({
      json: {
        commands: [
          { name: 'context', description: 'Show current context usage' },
          {
            name: 'effort',
            description: 'Set effort level for model usage',
            argument_hint: '<low|medium|high|xhigh|max|auto>',
          },
        ],
        models: [],
        output_styles: [],
      } satisfies Commands,
    }),
  );
  await page.goto(`/s/${RUNNING.id}`);
  const box = page.locator('app-composer textarea');
  await box.pressSequentially('/eff');
  await page.getByRole('option', { name: /\/effort/ }).click();
  await expect(box).toHaveValue('/effort ');
  // Straight on to its values, without typing again.
  const max = page.getByRole('option', { name: 'max' });
  await expect(max).toBeVisible();
  // The list itself: it lies over the conversation by design, as a sheet does.
  await expectCleanLayout(page, testInfo, { root: '.completion' });
  await max.click();
  await expect(box).toHaveValue('/effort max');
});

test('a word longer than the screen wraps inside the box rather than widening it @ phone width', async ({
  page,
}, testInfo) => {
  // A textarea sized by its content is as wide as its longest word: with nothing
  // to break at, the box and the send button left the screen.
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  const box = page.locator('app-composer textarea');
  await box.fill('a'.repeat(300));
  const width = page.viewportSize()?.width ?? 0;
  const pill = await page.locator('app-composer .box').boundingBox();
  expect(pill && pill.x + pill.width).toBeLessThanOrEqual(width);
  await expect(page.getByRole('button', { name: 'send' })).toBeInViewport({ ratio: 1 });
  await expectCleanLayout(page, testInfo, { root: 'app-composer' });
});

test('a message written offline waits in the conversation and goes, once, when back @ phone width', async ({
  page,
  context,
}, testInfo) => {
  await mockRunner(page);
  const sent: { text: string; id?: string }[] = [];
  await page.route('**/api/sessions/*/input', (r) => {
    sent.push(r.request().postDataJSON() as { text: string; id?: string });
    return r.fulfill({ json: RUNNING });
  });
  await page.goto(`/s/${RUNNING.id}`);
  await context.setOffline(true);
  const box = page.locator('app-composer textarea');
  await box.fill('check the build when you can');
  await page.getByRole('button', { name: 'send' }).click();

  // In the conversation, where it will stay once read; offline, it cannot have left.
  const mine = page.locator('li.entry', { hasText: 'check the build when you can' });
  await expect(mine).toContainText('waiting to send');
  await expect(box).toHaveValue('');
  await expectCleanLayout(page, testInfo, { allow: BUSY_BAR });
  await expectThumbTargets(page);
  expect(sent).toHaveLength(0);

  await context.setOffline(false);
  await expect(mine).toContainText('waiting to be read');
  expect(sent).toHaveLength(1);
  expect(sent[0]?.text).toBe('check the build when you can');
  expect(sent[0]?.id).toMatch(/^[0-9a-f-]{36}$/);
});

test('a message not yet read is taken back into the box @ phone width', async ({
  page,
  context,
}) => {
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  await context.setOffline(true);
  const box = page.locator('app-composer textarea');
  await box.fill('rename the branch');
  await page.getByRole('button', { name: 'send' }).click();
  await box.fill('and then');

  await page
    .locator('li.entry', { hasText: 'rename the branch' })
    .getByRole('button', { name: 'take back' })
    .click();
  // Its words first, what was being written after: nothing typed is lost.
  await expect(box).toHaveValue('rename the branch\n\nand then');
  await expect(page.locator('li.entry', { hasText: 'waiting to send' })).toHaveCount(0);
});

test('a message the session read first is not taken back, and says when @ phone width', async ({
  page,
}) => {
  await mockRunner(page);
  const at = Date.now() - 60_000;
  await page.route('**/api/sessions/*/input', (r) => r.fulfill({ json: RUNNING }));
  await page.route('**/api/sessions/*/withdraw', (r) =>
    r.fulfill({ status: 409, json: { why: 'read', at } satisfies Refusal }),
  );
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('app-composer textarea').fill('use the other branch');
  await page.getByRole('button', { name: 'send' }).click();

  const mine = page.locator('li.entry', { hasText: 'use the other branch' });
  await expect(mine).toContainText('waiting to be read');
  await mine.getByRole('button', { name: 'take back' }).click();
  await expect(page.getByText(/Not taken back: the session read it at/)).toBeVisible();
  await expect(page.locator('app-composer textarea')).toHaveValue('');
  // News, not a fault: it goes by itself rather than staying until the next send.
  await expect(page.getByText(/Not taken back/)).toHaveCount(0, { timeout: 10_000 });
});

test('a refused message stops its session until taken back @ phone width', async ({
  page,
}, testInfo) => {
  await mockRunner(page);
  await page.route('**/api/sessions/*/input', (r) =>
    r.fulfill({ status: 409, body: 'session is no longer accepting input' }),
  );
  await page.goto(`/s/${RUNNING.id}`);
  const box = page.locator('app-composer textarea');
  await box.fill('one more thing');
  await page.getByRole('button', { name: 'send' }).click();

  const refused = page.locator('li.entry', { hasText: 'one more thing' });
  await expect(refused).toContainText('not sent — session is no longer accepting input');
  await expectCleanLayout(page, testInfo, { allow: BUSY_BAR });
  await refused.getByRole('button', { name: 'take back' }).click();
  await expect(box).toHaveValue('one more thing');
});
