import {
  GOLDEN,
  NEXT,
  RUNNING,
  SHEET,
  STATE,
  expect,
  expectCleanLayout,
  expectNoClippedText,
  expectUpInTheBar,
  first,
  mockParse,
  mockRunner,
  nth,
  openParse,
  routeStream,
  settleTransforms,
  test,
  type Call,
  type EditRecord,
  type Overview,
  type Run,
  type Stretched,
  type Timed,
} from './console';

test.use({ serviceWorkers: 'block' });

test('a shell command opens as written and as read @ phone width', async ({ page }, testInfo) => {
  // The two halves have to be comparable without scrolling between them.
  // That is the whole reason they are stacked rather than switched between, and
  // it is a claim about a rendered page that no amount of reading the source can
  // settle: the raw text is unbreakable path-shaped strings and the parse below
  // it is a list of the same.
  await mockRunner(page);
  await mockParse(page);
  await page.goto(`/s/${RUNNING.id}`);
  await openParse(page);

  // The command entire, which the row it was opened from could only ellipsise.
  const raw = page.locator('.raw');
  await expect(raw).toContainText('nix develop -c lake build');
  await expect(raw).toContainText('tee /tmp/lean-gate.log');

  // And the parse under it, in the shell's own order.
  await expect(page.locator('.step')).toHaveCount(3);
  await expect(page.locator('.step .kind').nth(2)).toHaveText('write');

  // The claim the sheet is for. The call failed, so the write after the
  // `&&` may never have happened — and the summary says how many such uses there
  // are without hiding any of them.
  await expect(page.locator('.summary')).toContainText('2 unproven');
  await expect(page.locator('.used.unsure')).toHaveCount(2);

  await expectCleanLayout(page, testInfo, { root: SHEET });
});

test('both halves of a parsed command fit one screen @ phone width', async ({ page }) => {
  // The layout claim, measured rather than eyeballed: the raw text and the first
  // classified step have to be on screen together, or the sheet has failed at
  // the one thing it was stacked this way to do.
  await mockRunner(page);
  await mockParse(page);
  await page.goto(`/s/${RUNNING.id}`);
  await openParse(page);

  const together = await page.evaluate(() => {
    const raw = document.querySelector('.raw')?.getBoundingClientRect();
    const step = document.querySelector('.step')?.getBoundingClientRect();
    return raw && step ? { top: raw.top, bottom: step.bottom, screen: window.innerHeight } : null;
  });
  expect(together).not.toBeNull();
  expect(together!.top, 'the raw command starts above the fold').toBeLessThan(together!.screen);
  expect(
    together!.bottom,
    'the first step of the parse is off the bottom of the screen',
  ).toBeLessThanOrEqual(together!.screen);
});

test('a command that will not parse says so rather than looking empty @ phone width', async ({
  page,
}, testInfo) => {
  // 0.4% of the corpus's calls land here. An empty sheet would read as a command
  // that did nothing, which is the opposite of what it means.
  await mockRunner(page);
  // The longest label there is, because it renders inside a `<code>` and
  // that does not wrap by default. Invented phrases here ('a case arm') tested
  // a width nothing produces; this is `Reason::label`'s widest real answer.
  await mockParse(page, {
    error: 'parameter with an operator (${x:-y}, ${#x})',
    steps: [],
    unread: [],
    unrolled: 0,
    nested_unparsed: 0,
  });
  await page.goto(`/s/${RUNNING.id}`);
  await openParse(page);

  await expect(page.locator('.unread')).toContainText('does not parse');
  await expect(page.locator('.step')).toHaveCount(0);
  await expectCleanLayout(page, testInfo, { root: SHEET });
});

test('another machine is named on the step and on every path @ phone width', async ({
  page,
}, testInfo) => {
  // A path that looks local and is not is the mistake the whole separation
  // exists to prevent — so it is said twice, and a reader scrolling fast sees
  // it either way.
  await mockRunner(page);
  await mockParse(page, {
    steps: [
      {
        depth: 0,
        argv: ['ssh', 'root@isis.xinutec.org', 'cat /etc/nixos/configuration.nix'],
        reached: 'always',
        kind: 'remote',
        says: 'isis',
        scope: [],
        key: 'step-1',
        uses: [],
      },
      {
        depth: 1,
        host: 'isis',
        argv: ['cat', '/etc/nixos/configuration.nix'],
        reached: 'always',
        kind: 'read',
        uses: [
          {
            path: '/etc/nixos/configuration.nix',
            write: false,
            reached: 'sometimes',
            certain: false,
            host: 'isis',
          },
        ],
        scope: [],
        key: 'step-2',
        says: '',
      },
    ],
    unread: [],
    unrolled: 0,
    nested_unparsed: 0,
  });
  await page.goto(`/s/${RUNNING.id}`);
  await openParse(page);

  await expect(page.locator('.step').nth(1).locator('.host')).toContainText('isis');
  await expect(page.locator('.used .elsewhere')).toContainText('on isis');
  // Indented, so the wrapper is visibly not the thing that read the file.
  const indents = await page.evaluate(() =>
    [...document.querySelectorAll('.step')].map((s) => s.getBoundingClientRect().left),
  );
  expect(nth(indents, 1), 'the command inside the wrapper is not indented').toBeGreaterThan(
    first(indents),
  );
  await expectCleanLayout(page, testInfo, { root: SHEET });
});

test('an edit opens as a diff of what it replaced @ phone width', async ({ page }, testInfo) => {
  await mockRunner(page);
  await routeStream(page, (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: (
        [
          { kind: 'started', model: 'claude-opus-5[1m]', cwd: '/home/example/Code', tools: 14 },
          {
            kind: 'tool',
            id: 'e1',
            name: 'Edit',
            does: {
              kind: 'edit',
              path: '/home/example/Code/xinutec-infra/plan/core/src/intent.rs',
              before:
                "/// Only this fact's probes are dropped, on the assumption that an effect closing one\n/// fact does not disturb another.\nfn dropped() {}",
              after: "/// Only this fact's probes are dropped.\nfn dropped() {}",
              everywhere: false,
            },
            at: NEXT,
          },
          {
            kind: 'tool_result',
            id: 'e1',
            ok: true,
            detail: 'The file has been updated.',
            at: NEXT,
          },
        ] satisfies Timed[]
      )
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.goto(`/s/${RUNNING.id}`);

  await page.locator('.entry.tool button.opens').click();
  const sheet = page.locator('app-diff-sheet');
  await sheet.waitFor();
  await settleTransforms(page);
  await expect(sheet.locator('.who')).toHaveText('intent.rs');
  await expect(sheet.locator('.line.gone')).toHaveCount(2);
  await expect(sheet.locator('.line.added')).toHaveCount(1);
  await expect(sheet.locator('.line.same')).toContainText('fn dropped() {}');

  await expectCleanLayout(page, testInfo, { root: 'mat-bottom-sheet-container' });
  await expectNoClippedText(page, testInfo, 'app-diff-sheet');
});

test('what a command is predicted to change is marked on its row and drawn in its sheet @ phone width', async ({
  page,
}, testInfo) => {
  const command = "sed -i '' 's/one/two/' src/a.rs";
  await mockRunner(page);
  await page.route('**/api/sessions/*/edits', (r) =>
    r.fulfill({ json: { edited: [], diverged: [] } satisfies EditRecord }),
  );
  await routeStream(page, (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: (
        [
          { kind: 'started', model: 'claude-opus-5[1m]', cwd: '/home/example/Code', tools: 14 },
          { kind: 'tool', id: 'b1', name: 'Bash', does: { kind: 'bash', command }, at: NEXT },
          { kind: 'tool_result', id: 'b1', ok: true, detail: '', at: NEXT },
          {
            kind: 'edited',
            call: 'b1',
            hunks: [
              {
                path: '/home/example/Code/memview/src/a.rs',
                before: 'x\none\ny\n',
                after: 'x\ntwo\ny\n',
              },
            ],
          },
        ] satisfies Timed[]
      )
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.route('**/api/sessions/*/parse', (r) => r.fulfill({ json: GOLDEN }));
  await page.goto(`/s/${RUNNING.id}`);

  const row = page.locator('.entry.tool');
  await expect(row.locator('mat-icon.edited')).toBeVisible();
  await row.locator('button.opens').click();
  const sheet = page.locator('app-parse-sheet');
  await sheet.locator('app-diff').waitFor();
  await settleTransforms(page);
  await expect(sheet.locator('.file')).toContainText('a.rs');
  await expect(sheet.locator('.line.gone')).toHaveText('− one');
  await expect(sheet.locator('.line.added')).toHaveText('+ two');
  await expect(sheet.locator('.line.same')).toHaveCount(2);
  await expectCleanLayout(page, testInfo, { root: 'mat-bottom-sheet-container' });
});

test('a command waiting for permission shows what it will change @ phone width', async ({
  page,
}, testInfo) => {
  const command = "cat > notes.txt <<'EOF'\nnew\nEOF";
  await mockRunner(page);
  await page.route('**/api/sessions/*/edits', (r) =>
    r.fulfill({ json: { edited: [], diverged: [] } satisfies EditRecord }),
  );
  await routeStream(page, (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: (
        [
          { kind: 'started', model: 'claude-opus-5[1m]', cwd: '/home/example/Code', tools: 14 },
          { kind: 'tool', id: 'b1', name: 'Bash', does: { kind: 'bash', command }, at: NEXT },
          {
            kind: 'edited',
            call: 'b1',
            hunks: [{ path: '/home/example/Code/notes.txt', before: 'old\n', after: 'new\n' }],
          },
          {
            kind: 'ask',
            id: 'q1',
            call: 'b1',
            tool: 'Bash',
            does: { kind: 'bash', command },
            at: NEXT,
          },
        ] satisfies Timed[]
      )
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.route('**/api/sessions/*/parse', (r) => r.fulfill({ json: GOLDEN }));
  await page.goto(`/s/${RUNNING.id}`);

  const card = page.locator('app-ask-card');
  await expect(card.getByRole('button', { name: 'allow' })).toBeVisible();
  await card.locator('button.opens').click();
  const sheet = page.locator('app-parse-sheet');
  await expect(sheet.locator('.line.gone')).toHaveText('− old');
  await expect(sheet.locator('.line.added')).toHaveText('+ new');
  await settleTransforms(page);
  await expectCleanLayout(page, testInfo, { root: 'mat-bottom-sheet-container' });
});

test('a call whose files did not end up as predicted says so @ phone width', async ({
  page,
}, testInfo) => {
  await mockRunner(page);
  await page.route('**/api/sessions/*/edits', (r) =>
    r.fulfill({ json: { edited: [], diverged: [] } satisfies EditRecord }),
  );
  await routeStream(page, (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: (
        [
          { kind: 'started', model: 'claude-opus-5[1m]', cwd: '/home/example/Code', tools: 14 },
          {
            kind: 'tool',
            id: 'b1',
            name: 'Bash',
            does: { kind: 'bash', command: 'echo x > a' },
            at: NEXT,
          },
          {
            kind: 'edited',
            call: 'b1',
            hunks: [{ path: '/home/example/Code/a', before: '', after: 'x\n' }],
          },
          { kind: 'tool_result', id: 'b1', ok: true, detail: '', at: NEXT },
          { kind: 'diverged', call: 'b1', paths: ['/home/example/Code/a'] },
        ] satisfies Timed[]
      )
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.route('**/api/sessions/*/parse', (r) => r.fulfill({ json: GOLDEN }));
  await page.goto(`/s/${RUNNING.id}`);

  const row = page.locator('.entry.tool');
  await expect(row.locator('mat-icon.diverged')).toBeVisible();
  await expect(row.locator('mat-icon.edited')).toHaveCount(0);
  await row.locator('button.opens').click();
  const sheet = page.locator('app-parse-sheet');
  await expect(sheet.locator('.diverged')).toContainText('did not end up as predicted');
  await settleTransforms(page);
  await expectCleanLayout(page, testInfo, { root: 'mat-bottom-sheet-container' });
});

test('a prediction that assumed an unknown program harmless says which @ phone width', async ({
  page,
}, testInfo) => {
  const command = "python3 edit.py && scripts/dev bash -c 'cd lean && lake build'";
  await mockRunner(page);
  await page.route('**/api/sessions/*/edits', (r) =>
    r.fulfill({ json: { edited: [], diverged: [] } satisfies EditRecord }),
  );
  await routeStream(page, (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: (
        [
          { kind: 'started', model: 'claude-opus-5[1m]', cwd: '/home/example/Code', tools: 14 },
          { kind: 'tool', id: 'b1', name: 'Bash', does: { kind: 'bash', command }, at: NEXT },
          {
            kind: 'edited',
            call: 'b1',
            hunks: [
              {
                path: '/home/example/Code/health/lean/Verified/Hsmm/StationChain.lean',
                before: '  let fast := fun (k : Nat) => match moving[k]?.getD none with\n',
                after: '  let fast : Nat → Bool := fun k => match moving[k]?.getD none with\n',
                assumed: { before: [], after: ['dev'] },
              },
            ],
          },
          { kind: 'tool_result', id: 'b1', ok: true, detail: '', at: NEXT },
          {
            kind: 'diverged',
            call: 'b1',
            paths: ['/home/example/Code/health/lean/Verified/Hsmm/StationChain.lean'],
          },
        ] satisfies Timed[]
      )
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.route('**/api/sessions/*/parse', (r) => r.fulfill({ json: GOLDEN }));
  await page.goto(`/s/${RUNNING.id}`);

  // The sheet copies what the row holds when it opens: wait for the prediction.
  const row = page.locator('.entry.tool');
  await expect(row.locator('mat-icon.diverged')).toBeVisible();
  await row.locator('button.opens').click();
  const sheet = page.locator('app-parse-sheet');
  await expect(sheet.locator('.assumed')).toHaveText('If dev, run after, left it alone.');
  await expect(sheet.locator('.diverged')).toContainText('a program it assumed harmless');
  await settleTransforms(page);
  await page.screenshot({ path: testInfo.outputPath('conditional.png') });
  await expectCleanLayout(page, testInfo, { root: 'mat-bottom-sheet-container' });
});

test('a file one of several texts shows each outcome @ phone width', async ({ page }, testInfo) => {
  const command = 'if grep -q x f; then echo a > o; else echo b > o; fi';
  await mockRunner(page);
  await page.route('**/api/sessions/*/edits', (r) =>
    r.fulfill({ json: { edited: [], diverged: [] } satisfies EditRecord }),
  );
  await routeStream(page, (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: (
        [
          { kind: 'started', model: 'claude-opus-5[1m]', cwd: '/home/example/Code', tools: 14 },
          { kind: 'tool', id: 'b1', name: 'Bash', does: { kind: 'bash', command }, at: NEXT },
          {
            kind: 'edited',
            call: 'b1',
            hunks: [
              {
                path: '/home/example/Code/o',
                before: 'old\n',
                after: 'a\n',
                alternative: { at: 1, of: 2 },
              },
              {
                path: '/home/example/Code/o',
                before: 'old\n',
                after: 'b\n',
                alternative: { at: 2, of: 2 },
              },
            ],
          },
          { kind: 'tool_result', id: 'b1', ok: true, detail: '', at: NEXT },
        ] satisfies Timed[]
      )
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.route('**/api/sessions/*/parse', (r) => r.fulfill({ json: GOLDEN }));
  await page.goto(`/s/${RUNNING.id}`);

  const row = page.locator('.entry.tool');
  await expect(row.locator('mat-icon.edited')).toBeVisible();
  await row.locator('button.opens').click();
  const sheet = page.locator('app-parse-sheet');
  await expect(sheet.locator('.assumed')).toHaveText([
    'Outcome 1 of 2: which arm of an if runs is not known.',
    'Outcome 2 of 2: which arm of an if runs is not known.',
  ]);
  await settleTransforms(page);
  await page.screenshot({ path: testInfo.outputPath('outcomes.png') });
  await expectCleanLayout(page, testInfo, { root: 'mat-bottom-sheet-container' });
});

test('an edit waiting for permission can be read before it is allowed @ phone width', async ({
  page,
}, testInfo) => {
  const does = {
    kind: 'edit',
    path: '/home/example/Code/memview/console/src/usage.rs',
    before: 'const EVERY: Duration = Duration::from_secs(300);',
    after: 'const EVERY: Duration = Duration::from_secs(60);',
    everywhere: false,
  } satisfies Call;
  await mockRunner(page);
  await routeStream(page, (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: (
        [
          { kind: 'started', model: 'claude-opus-5[1m]', cwd: '/home/example/Code', tools: 14 },
          { kind: 'tool', id: 'e1', name: 'Edit', does, at: NEXT },
          {
            kind: 'ask',
            id: 'q1',
            call: 'e1',
            tool: 'Edit',
            does,
            title: 'Edit usage.rs',
            at: NEXT,
          },
        ] satisfies Timed[]
      )
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.goto(`/s/${RUNNING.id}`);

  const card = page.locator('app-ask-card');
  await expect(card.getByRole('button', { name: 'allow' })).toBeVisible();
  await card.locator('button.opens').click();
  const sheet = page.locator('app-diff-sheet');
  await sheet.waitFor();
  await settleTransforms(page);
  await expect(sheet.locator('.who')).toHaveText('usage.rs');
  await expect(sheet.locator('.line.gone')).toContainText('from_secs(300)');
  await expect(sheet.locator('.line.added')).toContainText('from_secs(60)');
  await expectCleanLayout(page, testInfo, { root: 'mat-bottom-sheet-container' });
});

test('the concept leads and the argv follows as evidence @ phone width', async ({
  page,
}, testInfo) => {
  // The wire carries a full resolved path; the card cuts it to the leaf, or one
  // path is stated several times on one step and pushes the command being
  // approved down the card. The path is not lost, and the last assertion below is what
  // says so: the use row still carries it, once, where direction and certainty
  // are marked.
  //
  // `describe` is unchanged and still returns the full path — it is the
  // CONCEPT's phrase, not this card's, and shortening it there would hand every
  // future consumer a 412px column.
  //
  // Drawn beside a command NO lens covers, on purpose: absent is the honest
  // miss, and the two have to be tellable apart at a glance or the layer is
  // claiming coverage it does not have.
  await mockRunner(page);
  await mockParse(page, {
    steps: [
      {
        depth: 0,
        argv: ['head', '-5', 'rust/backend/src/routes/session.rs'],
        reached: 'always',
        cwd: '/home/example/Code/health',
        concept:
          'Show the first 5 lines of /home/example/Code/health/rust/backend/src/routes/session.rs',
        kind: 'read',
        key: 'read',
        uses: [
          {
            path: '/home/example/Code/health/rust/backend/src/routes/session.rs',
            write: false,
            reached: 'always',
            certain: true,
          },
        ],
        scope: [],
        says: '',
      },
      {
        depth: 0,
        argv: ['wc', '-l', 'notes.md'],
        reached: 'always',
        cwd: '/home/example/Code/health',
        kind: 'read',
        key: 'read',
        says: 'counts lines',
        scope: [],
        uses: [],
      },
    ],
    unread: [],
    unrolled: 0,
    nested_unparsed: 0,
  });
  await page.goto(`/s/${RUNNING.id}`);
  await openParse(page);

  // Exactly one of the two steps says what it was for.
  await expect(page.locator('.concept')).toHaveCount(1);
  // The ACT, with the subject named compactly.
  await expect(page.locator('.concept')).toHaveText('Show the first 5 lines of session.rs');
  // The negative is the assertion. Containing the short form would still
  // pass if the sentence had the whole path in it, since the leaf is a suffix
  // of the path — so this names what must be ABSENT from the sentence.
  await expect(page.locator('.concept')).not.toContainText('/home/example');
  // And the path is still ON the card, which is what makes the cut safe
  // rather than a loss. It moved to the one row that also says direction and
  // certainty, instead of being repeated in prose above it.
  await expect(page.locator(SHEET)).toContainText(
    '/home/example/Code/health/rust/backend/src/routes/session.rs',
  );

  await expectCleanLayout(page, testInfo, { root: SHEET });
});

test('a workflow opens on its agents by phase, and an agent on its transcript @ phone width', async ({
  page,
}, testInfo) => {
  const run = 'wf_0480c7f6-60f';
  const task = 'wpzk9pgqb';
  const working = 'a3ce095293b0f6627';
  await mockRunner(page);
  await page.route('**/api/state', (r) =>
    r.fulfill({
      json: {
        ...STATE,
        sessions: STATE.sessions.map((one) =>
          one.id === RUNNING.id
            ? {
                ...one,
                background: 1,
                running: [{ tool: 'Workflow', label: 'comment-pass-1721', task }],
              }
            : one,
        ),
      } satisfies Overview,
    }),
  );
  await routeStream(page, (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: (
        [
          { kind: 'started', model: 'claude-opus-5[1m]', cwd: '/home/example/Code', tools: 14 },
          {
            kind: 'tool',
            id: 'w1',
            name: 'Workflow',
            does: { kind: 'workflow', name: 'comment-pass-1721' },
            at: NEXT,
          },
          {
            kind: 'tool_result',
            id: 'w1',
            ok: true,
            detail: `Workflow launched in background. Task ID: ${task}\nSummary: Rewrite long comment blocks\nRun ID: ${run}`,
            at: NEXT,
          },
        ] satisfies Timed[]
      )
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  const command =
    'cd /home/example/Code/xinutec-infra && cargo test --workspace --all-targets -- --test-threads=1 plan::runner::observe';
  await page.route(`**/api/sessions/*/workflows/${run}`, (r) =>
    r.fulfill({
      json: {
        phases: [
          {
            title: 'Rewrite',
            agents: [
              { id: 'af4359a23fcc0ae33', label: 'comments:core', done: true },
              {
                id: working,
                label: 'comments:runner',
                done: false,
                latest: {
                  kind: 'tool',
                  id: 't9',
                  name: 'Bash',
                  does: { kind: 'bash', command },
                  at: NEXT,
                },
              },
            ],
          },
        ],
      } satisfies Run,
    }),
  );
  await page.route(`**/api/sessions/*/workflows/${run}/agents/${working}*`, (r) =>
    r.fulfill({
      json: new URL(r.request().url()).searchParams.has('after')
        ? ({ events: [], from: 900, to: 900 } satisfies Stretched)
        : ({
            events: [
              { kind: 'prompt', text: 'You are doing part of a comment pass.', at: NEXT },
              { kind: 'text', text: 'Reading the runner first.', at: NEXT },
              {
                kind: 'tool',
                id: 't8',
                name: 'Read',
                does: {
                  kind: 'other',
                  shown: '/home/example/Code/xinutec-infra/plan/runner/src/act.rs',
                },
                at: NEXT,
              },
              { kind: 'tool_result', id: 't8', ok: true, detail: 'fn act() {}', at: NEXT },
              { kind: 'tool', id: 't9', name: 'Bash', does: { kind: 'bash', command }, at: NEXT },
            ],
            from: 0,
            to: 900,
          } satisfies Stretched),
    }),
  );
  await page.goto(`/s/${RUNNING.id}`);

  await page.locator('.entry.tool a.opens', { hasText: 'comment-pass-1721' }).click();
  await expect(page).toHaveURL(new RegExp(`/s/${RUNNING.id}/w/${run}\\?task=${task}`));
  const view = page.locator('app-workflow-view');
  await expect(view.locator('h2 .title')).toHaveText('Rewrite');
  await expect(view.locator('h2 .tally')).toHaveText('1/2 done');
  await expect(view.locator('.agent[data-state="done"] .label')).toHaveText('comments:core');
  const busy = view.locator('.agent[data-state="working"]');
  await expect(busy.locator('mat-spinner')).toBeVisible();
  await expect(busy.locator('.doing')).toContainText('Bash');
  await expect(page.locator('ui-scaffold h1')).toHaveText('comment-pass-1721');
  await expectUpInTheBar(page);
  await expectCleanLayout(page, testInfo);
  await expectNoClippedText(page, testInfo, 'app-workflow-view');
  await page.screenshot({ path: testInfo.outputPath('workflow.png') });

  await busy.click();
  const agent = page.locator('app-agent-view');
  await expect(agent.locator('.entry.asked .who')).toHaveText('workflow');
  await expect(agent.locator('.entry.said')).toContainText('Reading the runner first.');
  await expect(agent.locator('.entry.tools .count')).toContainText('2 tool calls');
  await expect(agent.locator('.entry.tools .running')).toContainText('1 running');
  await expect(page.locator('ui-scaffold h1')).toHaveText('comments:runner');
  await expectUpInTheBar(page);
  await expectCleanLayout(page, testInfo);
  await page.screenshot({ path: testInfo.outputPath('agent.png') });

  await page.getByLabel('back').click();
  await expect(page).toHaveURL(new RegExp(`/s/${RUNNING.id}/w/${run}\\?task=${task}`));
  await expect(view.locator('h2 .title')).toHaveText('Rewrite');
});
