import {
  BUSY_BAR,
  DECIDING,
  ENDED,
  NEXT,
  QUESTION_TRANSCRIPT,
  RUNNING,
  STATE,
  expect,
  expectCleanLayout,
  expectThumbTargets,
  first,
  last,
  mockQuestion,
  mockRunner,
  routeStream,
  test,
  type Overview,
  type SudoAnswer,
  type Summary,
  type Timed,
} from './console';

test.use({ serviceWorkers: 'block' });

test('a call waiting to be allowed is one widget, not two @ phone width', async ({
  page,
}, testInfo) => {
  // Reported from the phone, diagnosed by driving a real session.
  // The CLI emits `tool toolu_…` and then `ask …` about the same call, and the
  // console drew both: a tool row AND a permission card for one Write. The card
  // also sat between the calls either side of it, so a sequence of decided calls
  // could never fold into one list.
  await mockRunner(page);
  await routeStream(page, (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: DECIDING.map((event) => `data: ${JSON.stringify(event)}\n\n`).join(''),
    }),
  );
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('.question').waitFor();

  // ONE widget for the call being asked about: the question, and no row of its
  // own underneath it.
  await expect(page.locator('.question')).toHaveCount(1);
  await expect(page.getByText('notes.md')).toHaveCount(1);

  // And the two calls before it are a folded run, which they could not be while
  // a card stood between them and the newest call.
  const run = page.locator('.entry.tools .run');
  await expect(run).toHaveCount(1);
  await expect(run).toContainText('2 tool calls');

  // The widest thing this page holds: a whole path, a question, and two buttons
  // on one 412px line.
  await expectCleanLayout(page, testInfo, { allow: BUSY_BAR });
  await expectThumbTargets(page);
});

test('a root password is asked for, and nothing is sent until one is typed @ phone width', async ({
  page,
}) => {
  // Tapping run on an empty field sent sudo an empty password twice: a refusal,
  // and a wrong attempt counted against the account.
  let sent: SudoAnswer | undefined;
  await mockRunner(page);
  const transcript = [
    { kind: 'started', model: 'claude-opus-5[1m]', cwd: '/home/example/Code/memview', tools: 14 },
    {
      kind: 'ask',
      id: 'sudo-1',
      tool: 'sudo',
      does: { kind: 'bash', command: 'launchctl kickstart -k system/org.nixos.nix-daemon' },
      at: NEXT,
    },
  ] satisfies Timed[];
  await routeStream(page, (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: transcript.map((event) => `data: ${JSON.stringify(event)}\n\n`).join(''),
    }),
  );
  await page.route('**/api/sessions/*/sudo', (r) => {
    sent = r.request().postDataJSON() as SudoAnswer;
    return r.fulfill({ status: 204, body: '' });
  });
  await page.goto(`/s/${RUNNING.id}`);
  const field = page.getByLabel('password for sudo');
  const run = page.getByRole('button', { name: 'run' });
  await expect(page.getByText('launchctl kickstart -k system/org.nixos.nix-daemon')).toBeVisible();
  await expect(run).toBeDisabled();
  await field.press('Enter');
  expect(sent, 'Enter on an empty field sends nothing').toBeUndefined();

  await field.fill('hunter2');
  await expect(run).toBeEnabled();
  await run.click();
  await expect.poll(() => sent).toEqual({ id: 'sudo-1', password: 'hunter2' });
  await expect(field).toHaveValue('');
});

test('a question offers what was asked, not allow and refuse @ phone width', async ({
  page,
}, testInfo) => {
  // The defect this was built for. The console showed a question as a
  // permission for months: the options arrived in the ask and were never
  // rendered, so the only answer it could give was an approval with no answer in
  // it — which the CLI reports as "the user did not answer the questions".
  const sent = await mockQuestion(page);
  await page.goto(`/s/${RUNNING.id}`);
  await page.getByRole('button', { name: /options only/ }).waitFor();

  await expect(
    page.getByRole('button', { name: 'allow' }),
    'a question is not a permission',
  ).toHaveCount(0);
  // The description is the part worth reading, and the reason these are not
  // Material buttons — its label spills rather than wrapping.
  await expect(page.locator('.option .means').first()).toContainText('Smallest change');
  await expectCleanLayout(page, testInfo, { allow: BUSY_BAR });
  await expectThumbTargets(page);

  // Two questions stand, so one tap is not an answer: it is remembered, and the
  // send button stays out of reach until the other is answered too.
  await page.getByRole('button', { name: /options only/ }).click();
  expect(sent(), 'sent before the second question was answered').toBeUndefined();
  await expect(page.getByRole('button', { name: 'answer', exact: true })).toBeDisabled();

  await page.getByRole('button', { name: /the description/ }).click();
  await page.getByRole('button', { name: /the topic/ }).click();
  await page.getByRole('button', { name: 'answer', exact: true }).click();

  await expect.poll(sent).toMatchObject({
    allow: true,
    answers: {
      'How far should the question UI go?': 'options only',
      // Both, in the order they were tapped — a multi-select question that came
      // back as a single label would silently drop a choice.
      'Which of these should the card show?': ['the description', 'the topic'],
    },
  });
});

test('an answered question says what was chosen @ phone width', async ({ page }, testInfo) => {
  // The verdict arrives from the runner, not from whichever screen tapped — so
  // this drives it the way a SECOND window would see it, with no local state to
  // fall back on. That is the property worth pinning: `answered` on its own
  // makes the card forget the thing you just did.
  await mockRunner(page);
  await routeStream(page, (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: (
        [
          ...QUESTION_TRANSCRIPT,
          {
            kind: 'answered',
            id: 'c9f0a1b2-0000-4000-8000-00000000000a',
            allowed: true,
            reply: {
              answers: {
                'How far should the question UI go?': 'options only',
                'Which of these should the card show?': ['the description', 'the topic'],
              },
            },
          },
        ] satisfies Timed[]
      )
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('.chose').waitFor();

  // Not yet 'answered': the write reaching the pipe is not the session reading
  // it, and one that has stopped reading stays blocked. Nothing has come back from
  // the session in this stream, so
  // the honest state is the intermediate one.
  await expect(page.locator('.verdict')).toHaveText('sent — not taken up yet');
  await expect(page.locator('.chose')).toHaveText('options only · the description, the topic');
  // Nothing left to tap: the question is over.
  await expect(page.getByRole('button', { name: /options only/ })).toHaveCount(0);
  await expectCleanLayout(page, testInfo, { allow: BUSY_BAR });
});

test('a typed reply is recorded as one, not as a choice @ phone width', async ({ page }) => {
  await mockRunner(page);
  await routeStream(page, (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: (
        [
          ...QUESTION_TRANSCRIPT,
          {
            kind: 'answered',
            id: 'c9f0a1b2-0000-4000-8000-00000000000a',
            allowed: true,
            reply: { response: 'neither — do the read-only part first' },
          },
        ] satisfies Timed[]
      )
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('.chose').waitFor();
  // Sent, not yet taken up — see the note in the test above.
  await expect(page.locator('.verdict')).toHaveText('sent — not taken up yet');
  await expect(page.locator('.chose')).toHaveText('neither — do the read-only part first');
});

test('a note rides with the choice it qualifies @ phone width', async ({ page }, testInfo) => {
  // The difference from the reply field. Words override the choices; a
  // note qualifies one, so both must arrive — and the options must stay live
  // while one is being written.
  const sent = await mockQuestion(page);
  await page.goto(`/s/${RUNNING.id}`);
  const options = page.getByRole('button', { name: /options only/ });
  await options.waitFor();

  await page.getByRole('button', { name: 'add a note' }).first().click();
  await page
    .getByRole('textbox', { name: /a note about How far/ })
    .fill('but keep the skip button');
  await expect(options, 'a note is not a reply and must not take the card over').toBeEnabled();
  await options.click();
  await page.getByRole('button', { name: /the description/ }).click();
  await expectCleanLayout(page, testInfo, { allow: BUSY_BAR });
  await expectThumbTargets(page);
  await page.getByRole('button', { name: 'answer', exact: true }).click();

  await expect.poll(sent).toMatchObject({
    answers: { 'How far should the question UI go?': 'options only' },
    annotations: { 'How far should the question UI go?': { notes: 'but keep the skip button' } },
  });
});

test('a note alone is enough to send @ phone width', async ({ page }) => {
  // The CLI records this as `(no option selected) notes: …` and treats it as
  // answered, so a card that waited for a tap would sit grey over something the
  // session would have taken.
  const sent = await mockQuestion(page);
  await page.goto(`/s/${RUNNING.id}`);
  await page.getByRole('button', { name: /options only/ }).waitFor();

  for (const which of [/a note about How far/, /a note about Which of these/]) {
    await page.getByRole('button', { name: 'add a note' }).first().click();
    await page.getByRole('textbox', { name: which }).fill('ask me again tomorrow');
  }
  const send = page.getByRole('button', { name: 'answer', exact: true });
  await expect(send).toBeEnabled();
  await send.click();

  await expect.poll(sent).toMatchObject({
    annotations: {
      'How far should the question UI go?': { notes: 'ask me again tomorrow' },
      'Which of these should the card show?': { notes: 'ask me again tomorrow' },
    },
  });
});

test('words instead of a choice take the card over @ phone width', async ({ page }, testInfo) => {
  // The trap this shape exists to close. The CLI's result builder tests
  // `response` before `answers` and reports only what it finds, so words sent
  // alongside a set of taps would throw the taps away and say nothing about it.
  // Typing therefore disables the options rather than sitting beside them.
  const sent = await mockQuestion(page);
  await page.goto(`/s/${RUNNING.id}`);
  const options = page.getByRole('button', { name: /options only/ });
  await options.waitFor();
  await expect(options).toBeEnabled();

  await page.locator('.say').first().fill('neither — do the read-only part first');
  await expect(options, 'an option that could still be tapped but would not arrive').toBeDisabled();

  const send = page.getByRole('button', { name: 'reply', exact: true });
  await expect(send, 'the button says what it will do').toBeVisible();
  await expectCleanLayout(page, testInfo, { allow: BUSY_BAR });
  await expectThumbTargets(page);
  await send.click();

  await expect.poll(sent).toMatchObject({
    allow: true,
    response: 'neither — do the read-only part first',
  });
  // And nothing was picked, so nothing pretends to have been.
  expect(sent()?.['answers'], 'answers rode along and would have been discarded').toBeUndefined();
  // Nor notes: a note qualifies a choice, and no choice is being sent.
  expect(sent()?.['annotations']).toBeUndefined();
});

test('clearing the words hands the options back @ phone width', async ({ page }) => {
  await mockQuestion(page);
  await page.goto(`/s/${RUNNING.id}`);
  const options = page.getByRole('button', { name: /options only/ });
  await options.waitFor();
  await page.locator('.say').first().fill('actually, never mind');
  await expect(options).toBeDisabled();
  // Whitespace is not an answer either — the CLI trims before testing it.
  await page.locator('.say').first().fill('   ');
  await expect(options).toBeEnabled();
  await expect(page.getByRole('button', { name: 'reply', exact: true })).toHaveCount(0);
});

test('a lone single-choice question answers on the tap @ phone width', async ({ page }) => {
  // One tap, because the alternative is two: tap the option, then reach for a
  // send button. On a phone that difference is whether it gets answered from the
  // lock screen or put off until later.
  let sent: Record<string, unknown> | undefined;
  await mockRunner(page);
  const asking = last(QUESTION_TRANSCRIPT);
  if (asking.kind !== 'ask' || asking.does.kind !== 'question') {
    throw new Error('the question transcript ends in a question');
  }
  const alone = {
    ...asking,
    does: {
      ...asking.does,
      questions: [first(asking.does.questions)],
    },
  };
  await routeStream(page, (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: ([first(QUESTION_TRANSCRIPT), alone] satisfies Timed[])
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.route('**/api/sessions/*/decide', (r) => {
    sent = r.request().postDataJSON() as Record<string, unknown>;
    return r.fulfill({ json: RUNNING });
  });

  await page.goto(`/s/${RUNNING.id}`);
  await page.getByRole('button', { name: /full parity/ }).click();
  await expect
    .poll(() => sent)
    .toMatchObject({
      answers: { 'How far should the question UI go?': 'full parity' },
    });
  // And no send button was ever offered, because there was nothing to wait for.
  await expect(page.getByRole('button', { name: 'answer', exact: true })).toHaveCount(0);
});

test('transcript — an undecided question with its two buttons @ phone width', async ({
  page,
}, testInfo) => {
  // The one screen that must work under a thumb on a train: a long command and
  // two controls, on a narrow screen, with nothing pushed off the edge.
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  await page.getByRole('button', { name: 'allow' }).waitFor();
  await expectCleanLayout(page, testInfo, { allow: BUSY_BAR });
  // The decision and the send button on one screen: the controls whose size is
  // the difference between answering from a train and waiting until you are off
  // it.
  await expectThumbTargets(page);
});

test('the verdict becomes the plain one once the session acts on it @ phone width', async ({
  page,
}, testInfo) => {
  // *Sent — not taken up yet* has to actually resolve.
  // The receipt is the session speaking, because a question blocks the turn — so
  // one tool call after the answer is proof it was read.
  await mockRunner(page);
  await routeStream(page, (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: (
        [
          ...QUESTION_TRANSCRIPT,
          {
            kind: 'answered',
            id: 'c9f0a1b2-0000-4000-8000-00000000000a',
            allowed: true,
            reply: { answers: { 'How far should the question UI go?': 'options only' } },
          },
          {
            kind: 'tool',
            id: 'toolu_after',
            name: 'Bash',
            does: { kind: 'bash', command: 'echo taken up' },
          },
        ] satisfies Timed[]
      )
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('.chose').waitFor();

  await expect(page.locator('.verdict')).toHaveText('answered');
  await expectCleanLayout(page, testInfo, { allow: BUSY_BAR });
});

test('a refused mode change says so and puts the mode back @ phone width', async ({ page }) => {
  // A claim, not a crash: a refused `bypassPermissions` must not leave the header
  // saying Bypass Permissions over a session still asking for approval. The
  // refusal arrives on the poll, not on the request.
  const REFUSED =
    'Cannot set permission mode to bypassPermissions because the session was not ' +
    'launched with --dangerously-skip-permissions';
  await mockRunner(page);
  // What the runner sends once its own `settle_mode` has put the mode back: the
  // true mode, and the CLI's words for why the other one did not take.
  const settled = { ...RUNNING, mode: 'auto', mode_refused: REFUSED } satisfies Summary;
  await page.route('**/api/state', (r) =>
    r.fulfill({ json: { ...STATE, sessions: [settled, ENDED] } satisfies Overview }),
  );
  await page.goto(`/s/${RUNNING.id}`);

  // Said out loud, because the mode lives in a menu that is shut by the time the
  // answer comes back — a correction nobody can see is the defect over again.
  const snack = page.locator('mat-snack-bar-container');
  await expect(snack, 'the refusal was never shown').toContainText(
    '--dangerously-skip-permissions',
  );

  // And the header agrees with the session rather than with the request.
  await page
    .locator('ui-scaffold')
    .getByRole('button', { name: /what to do with/ })
    .click();
  await expect(page.locator('.session-menu .current')).toHaveText('Auto');
});

test('the permission modes are one row that opens a sheet @ phone width', async ({
  page,
}, testInfo) => {
  // Six modes and a heading were most of the menu, so Details and Tasks — the
  // two things it is actually opened for — sat above a wall of settings.
  await mockRunner(page);
  // With a mode set, which every real session has — the runner records one at
  // spawn. The row shows it, so the state is readable without opening anything.
  const onAuto = { ...RUNNING, mode: 'acceptEdits' } satisfies Summary;
  await page.route('**/api/state', (r) =>
    r.fulfill({ json: { ...STATE, sessions: [onAuto, ENDED] } satisfies Overview }),
  );
  await page.goto(`/s/${RUNNING.id}`);
  await page
    .locator('ui-scaffold')
    .getByRole('button', { name: /what to do with/ })
    .click();
  await expect(page.locator('.session-menu .current')).toHaveText('Accept edits');
  await expect(page.getByRole('menuitem', { name: /Asks permission/ })).toBeVisible();
  // Rename must not wear the `acceptEdits` pencil, which is the row
  // directly below it — the same glyph a thumb apart, meaning two unrelated
  // things. Reported from the phone.
  const icon = (name: RegExp) =>
    page.getByRole('menuitem', { name }).locator('mat-icon').first().textContent();
  expect(await icon(/Rename/)).not.toBe(await icon(/Asks permission/));
  await page.getByRole('menuitem', { name: /Asks permission/ }).click();
  const sheet = page.locator('mat-bottom-sheet-container');
  await sheet.waitFor();
  await expect(sheet.locator('.mode')).toHaveCount(6);
  // Waiting on the last row being IN the viewport is what says the sheet has
  // finished arriving. Not because the slide is invisible to `getAnimations` —
  // it is not, see [[openParse]] — but because this row has to be reachable, not
  // merely still: a sheet that has come to rest with its last mode below the
  // fold is the defect this line is here for.
  await expect(sheet.locator('.mode').last()).toBeInViewport();
  await expectCleanLayout(page, testInfo, { root: 'mat-bottom-sheet-container' });
  await expectThumbTargets(page);
});
