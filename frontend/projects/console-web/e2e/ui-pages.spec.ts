import { expect, test, type Page } from '@playwright/test';
// The fleet-shared harness, published as @xinutec/ui-harness (source repo
// ~/Code/ui-harness). Ships compiled JS, so it loads straight from node_modules.
import {
  expectIconFontLoaded,
  expectNoHorizontalOverflow,
  expectNoTextOverlaps,
  expectViewportIsPhone,
} from '@xinutec/ui-harness';

/**
 * Phone-width layout harness for the console. Render the real screens at a Pixel
 * viewport with the runner mocked and BUSY data, and assert the failure classes
 * that read fine in source and only show on a real phone.
 *
 * The at-risk strings here are not prose. A session is named by a filesystem
 * path with no spaces in it; a transcript is a column of tool arguments that are
 * paths, shell command lines and long flags. Those are the things that push a
 * page sideways, and this app puts them on a 412px screen on purpose.
 */
test.use({ serviceWorkers: 'block' });

/** Material's indeterminate progress bar animates by translating its two bars
 *  right across the viewport and off the left edge — that is how the animation
 *  works, not a layout fault, and it is only on screen while a session is
 *  working. Allowed by selector rather than by turning the check off, so a real
 *  overflow on the same page still fails. */
const BUSY_BAR = ['mat-progress-bar'];

/** The Material target size, and the size Material's own buttons are not. */
const THUMB = 48;

/**
 * Every control a thumb has to hit, and whether it can be hit.
 *
 * This page is driven one-handed on a moving train, so the size of a control is
 * a functional property rather than a styling one — and it is not readable from
 * the source: Material sets button height through its own tokens, so what a
 * control ends up as is only knowable from a rendered box.
 *
 * Buttons and links only. A `summary` — the disclosure on a tool result — is
 * deliberately left out and kept compact instead: a transcript holds dozens of
 * tool calls, and 48px each would cost a screenful of dead space in the reading
 * that is this page's main job, against a mis-tap whose worst outcome is opening
 * the wrong result.
 */
async function expectThumbTargets(page: Page, min = THUMB): Promise<void> {
  const small = await page.evaluate((least) => {
    const missed: { label: string; width: number; height: number }[] = [];
    for (const control of document.querySelectorAll('button, a[href]')) {
      const box = control.getBoundingClientRect();
      // Not rendered at all — a disabled control in a collapsed branch.
      if (box.width === 0 && box.height === 0) continue;
      if (box.height >= least && box.width >= least) continue;
      missed.push({
        label: (control.textContent ?? '').trim().slice(0, 40) || control.className,
        width: Math.round(box.width),
        height: Math.round(box.height),
      });
    }
    return missed;
  }, min);
  expect(small, `controls smaller than ${min}px in either direction`).toEqual([]);
}

const REPOS = [
  '/home/example/Code/health',
  '/home/example/Code/memview',
  '/home/example/Code/nixos-config',
];

/** A roster with the shapes that crowd a narrow screen: a deep path, a long
 *  first instruction, a session that is working and one that has ended. */
const STATE = {
  dirs: ['/home/example/Code'],
  repos: REPOS,
  sessions: [
    {
      id: '6f7c2f11-0000-4000-8000-000000000001',
      dir: '/home/example/Code/health/packages/health-sync-backend/src/decode',
      started: 1785600000,
      alive: true,
      model: 'claude-opus-5[1m]',
      busy: 'requesting',
      turns: 12,
      cost_usd: 4.2137,
      waiting: 1,
      asked:
        'Port the remaining matcher gate to Lean and prove it bit-exact against the TypeScript quant twin, then run the golden set and report which journeys moved.',
    },
    {
      id: '6f7c2f11-0000-4000-8000-000000000002',
      dir: '/home/example/Code/memview',
      started: 1785599000,
      alive: false,
      model: 'claude-haiku-4-5-20251001',
      turns: 3,
      cost_usd: 0.0084,
      waiting: 0,
      asked: 'check the corpus',
    },
  ],
};

/**
 * The same roster, long enough that the list cannot fit on a phone.
 *
 * ⚠ **The two-session fixture above is a screen and a half short of the defect.**
 * The console drives a dozen sessions at once — that is what it is for — and
 * every existing check passed a list that was quietly painting over the build
 * stamp, because no fixture had ever made the page taller than the viewport.
 */
const CROWDED = {
  ...STATE,
  sessions: Array.from({ length: 12 }, (_, index) => ({
    ...STATE.sessions[index % 2],
    id: `6f7c2f11-0000-4000-8000-0000000${String(index + 10).padStart(5, '0')}`,
    dir: REPOS[index % REPOS.length],
  })),
};

/** Two fixed moments either side of midnight, so the date row between them is on
 *  screen for every run rather than on the runs that happen to straddle one. */
const LATE = new Date(2026, 6, 30, 23, 52).getTime();
const NEXT = new Date(2026, 6, 31, 8, 14).getTime();

/** What a `Read` of a long file comes back as: the runner cuts it, and the page
 *  has to hold the cut without widening — the lines are unwrapped source. */
const LONG_RESULT = Array.from(
  { length: 30 },
  (_, line) =>
    `   ${line + 1}\texport const quantiseLegCost = (leg: MatcherLeg, budget: number): number =>`,
).join('\n');

/** A transcript with every entry kind, and the arguments that are hardest to
 *  fit: an absolute path, a piped shell command, a failed call, a cut tool
 *  result, and an answer carrying a fenced code block of unbreakable lines. */
const TRANSCRIPT = [
  { kind: 'started', model: 'claude-opus-5[1m]', cwd: STATE.sessions[0].dir, tools: 30, at: LATE },
  {
    kind: 'prompt',
    text: 'Port the remaining matcher gate to Lean and prove it bit-exact against the TypeScript quant twin.',
    at: LATE,
  },
  {
    kind: 'tool',
    id: 'toolu_01',
    name: 'Read',
    input: {
      file_path:
        '/home/example/Code/health/packages/health-sync-backend/src/decode/matcher-gate.ts',
    },
    at: NEXT,
  },
  // A result the runner had to cut: unwrapped source, which is the widest single
  // thing this page can be asked to hold.
  { kind: 'tool_result', id: 'toolu_01', ok: true, detail: LONG_RESULT, cut: 48213 },
  {
    kind: 'tool',
    id: 'toolu_02',
    name: 'Bash',
    input: {
      command:
        'nix develop -c lake build && ./verified_cli match --serve --timeout 30000ms | tee /tmp/lean-gate.log',
    },
  },
  // A failure, whose result the page opens by default — the one somebody
  // scrolled back to read.
  {
    kind: 'tool_result',
    id: 'toolu_02',
    ok: false,
    detail: 'error: unknown flag --serve\nnote: run with --help for a list\nexit status 2',
  },
  {
    kind: 'text',
    text: 'The build fails on `quantiseLegCost`: the Lean version rounds half-to-even and ',
  },
  {
    kind: 'text',
    text: 'the TypeScript one rounds half-away-from-zero, so the two disagree on exactly the ties.',
  },
  // An answer carrying code. A fenced block is one unbreakable line, and it is
  // the commonest thing an answer holds that a 412px screen cannot fit.
  {
    kind: 'text',
    text: '\n\n```ts\nconst quantiseLegCost = (leg: MatcherLeg, budget: number): number => Math.round(leg.seconds * budget * QUANT_SCALE) / QUANT_SCALE;\n```\n',
  },
  { kind: 'turn', cost_usd: 0.3312, turns: 1, duration_ms: 48210, at: NEXT },
  // A question, undecided: the widest thing on the page, since it carries a
  // whole command AND two buttons on one 412px line.
  {
    kind: 'ask',
    id: '8ed3af09-323c-404b-8368-9682dca75d26',
    tool: 'Bash',
    title: 'Claude wants to run nix develop -c lake build --verbose 2>&1 | tee /tmp/lean.log',
    input: { command: 'nix develop -c lake build --verbose 2>&1 | tee /tmp/lean.log' },
    at: NEXT,
  },
];

/** Mock every backend call. Catch-all FIRST — Playwright runs handlers
 *  last-registered-first. The event stream is served as a complete SSE body, so
 *  the transcript renders without a live runner. */
async function mockRunner(page: Page): Promise<void> {
  await page.route('**/api/**', (r) =>
    r.request().method() === 'GET' ? r.fulfill({ json: [] }) : r.fulfill({ status: 204, body: '' }),
  );
  await page.route('**/api/state', (r) => r.fulfill({ json: STATE }));
  await page.route('**/api/sessions/*/events', (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: TRANSCRIPT.map((event) => `data: ${JSON.stringify(event)}\n\n`).join(''),
    }),
  );
}

/**
 * Nothing pinned may sit on top of anything else pinned.
 *
 * ⚠ **Not covered by the text-overlap check**, which is why this exists as its
 * own assertion. That one compares text nodes, and the composer's biggest
 * surface is an empty `textarea` with no text in it at all — so a footer
 * rendering straight through the text box was invisible to it, while being the
 * first thing anybody saw on the phone. Boxes, not words.
 */
async function expectNoPinnedOverlap(page: Page): Promise<void> {
  const clashes = await page.evaluate(() => {
    const boxes = ['.composer', '.build', '.bar'].map((sel) => ({
      sel,
      box: document.querySelector(sel)?.getBoundingClientRect(),
    }));
    const bad: string[] = [];
    for (let i = 0; i < boxes.length; i++) {
      for (let j = i + 1; j < boxes.length; j++) {
        const a = boxes[i].box;
        const b = boxes[j].box;
        if (!a || !b) continue;
        const over =
          Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top) > 1 &&
          Math.min(a.right, b.right) - Math.max(a.left, b.left) > 1;
        if (over) bad.push(`${boxes[i].sel} over ${boxes[j].sel}`);
      }
    }
    return bad;
  });
  expect(clashes, 'pinned regions overlap').toEqual([]);
}

/**
 * The composer spans the width it is given.
 *
 * The inverse of the overflow check, and its blind spot: that one only ever
 * catches a page too *wide*. A composer inset by its container's padding and
 * capped again by its own `max-width` was a narrow strip in the middle of a
 * 412px screen, and every existing assertion passed.
 */
async function expectComposerFillsTheWidth(page: Page): Promise<void> {
  const width = await page.evaluate(() => {
    const composer = document.querySelector('.composer')?.getBoundingClientRect();
    const field = document.querySelector('.composer .box')?.getBoundingClientRect();
    return composer && field
      ? { composer: composer.width, field: field.width, page: window.innerWidth }
      : null;
  });
  expect(width, 'no composer on this page').not.toBeNull();
  // The composer reaches both edges; the text box takes what is left after the
  // send button, which is the only other thing on the row.
  expect(width!.composer).toBe(width!.page);
  expect(width!.field).toBeGreaterThan(width!.page * 0.7);
}

/**
 * The send button sits on the same line as the box it sends.
 *
 * ⚠ Not a cosmetic assertion. A `mat-form-field` reserves a line of hint/error
 * space below its input by default — `.mat-mdc-form-field-bottom-align::before`
 * in Material's own CSS — so a row aligned to `flex-end` aligns its other
 * controls to the bottom of a box whose last line is invisible. The button then
 * sits a full subscript line low, and every existing check passes it: it is not
 * clipped, not small, not overflowing, and does not overlap anything.
 */
async function expectSendAlignsWithTheBox(page: Page): Promise<void> {
  const rows = await page.evaluate(() => {
    // The OUTLINE's box, not the textarea's: the field pads its own text, so a
    // textarea bottom is inset from the visible edge the eye lines up against.
    const input = document
      .querySelector('.composer .mat-mdc-text-field-wrapper')
      ?.getBoundingClientRect();
    const send = document.querySelector('.composer .send')?.getBoundingClientRect();
    return input && send
      ? { inputBottom: input.bottom, sendBottom: send.bottom, sendHeight: send.height }
      : null;
  });
  expect(rows, 'no composer on this page').not.toBeNull();
  // Bottoms within a few pixels: the button is a fixed-height control beside a
  // box that grows, and `align-items: flex-end` is what keeps them together.
  expect(Math.abs(rows!.sendBottom - rows!.inputBottom)).toBeLessThan(8);
}

/**
 * The stamp at the foot of the shell is not painted over by the page above it.
 *
 * ⚠ **A different failure from `expectNoPinnedOverlap`**, which compares the
 * pinned regions with each other and never looks at the content. The shell is a
 * `100dvh` column of toolbar, page and stamp, and `.page` is deliberately
 * shrinkable — `min-height: 0` is what lets a flex item be shorter than what is
 * inside it. Shorter, with `overflow: visible`, means the content is neither
 * clipped nor scrolled: it spills out of the box and paints over the next row.
 * So the check has to be against what the page actually holds.
 */
async function expectTheStampIsClear(page: Page): Promise<void> {
  const clashes = await page.evaluate(() => {
    const stamp = document.querySelector('.build')?.getBoundingClientRect();
    if (!stamp) return ['no build stamp on this page'];

    // ⚠ **What is painted, not what is laid out.** A card scrolled halfway out
    // of a scrolling region still reports its whole box from
    // `getBoundingClientRect` — the clipping is done by the ancestor, and it is
    // the difference between the defect and the fix: the same card, at the same
    // coordinates, is either over the stamp or cut off above it depending on one
    // `overflow` declaration further up. So each box is intersected with every
    // ancestor that actually clips.
    const painted = (node: Element): DOMRect => {
      let box = node.getBoundingClientRect();
      for (let up = node.parentElement; up; up = up.parentElement) {
        const style = getComputedStyle(up);
        if (style.overflowY === 'visible' && style.overflowX === 'visible') continue;
        const clip = up.getBoundingClientRect();
        box = new DOMRect(
          Math.max(box.left, clip.left),
          Math.max(box.top, clip.top),
          Math.max(0, Math.min(box.right, clip.right) - Math.max(box.left, clip.left)),
          Math.max(0, Math.min(box.bottom, clip.bottom) - Math.max(box.top, clip.top)),
        );
      }
      return box;
    };

    const bad: string[] = [];
    // The cards, not every node: a box that overlaps is reported once, by the
    // thing somebody can see, rather than once per span inside it.
    for (const card of document.querySelectorAll('.page .session, .page .start, .page .past')) {
      const box = painted(card);
      const over =
        Math.min(box.bottom, stamp.bottom) - Math.max(box.top, stamp.top) > 1 &&
        Math.min(box.right, stamp.right) - Math.max(box.left, stamp.left) > 1;
      if (over) {
        bad.push(
          `${card.className.split(' ')[0]} [${Math.round(box.top)}–${Math.round(box.bottom)}]` +
            ` over .build [${Math.round(stamp.top)}–${Math.round(stamp.bottom)}]`,
        );
      }
    }
    return bad;
  });
  expect(clashes, 'the page paints over the build stamp').toEqual([]);
}

// The checker-checker: fail loudly here if the device preset is ever lost and
// the "phone width" suite silently runs at desktop width.
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
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo);
  await expectThumbTargets(page);
});

test('session list — a dozen sessions do not reach the build stamp @ phone width', async ({
  page,
}, testInfo) => {
  await mockRunner(page);
  await page.route('**/api/state', (r) => r.fulfill({ json: CROWDED }));
  await page.goto('/');
  await expect(page.locator('.page .session')).toHaveCount(CROWDED.sessions.length);
  await expectTheStampIsClear(page);
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo);
});

test('transcript — tool arguments and a fixed composer @ phone width', async ({
  page,
}, testInfo) => {
  await mockRunner(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
  // The failed shell call is the widest thing on the page; wait for it rather
  // than for the first paint, or the checks run against half a transcript.
  await page.getByText('verified_cli').first().waitFor();
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo, null, BUSY_BAR);
  await expectNoPinnedOverlap(page);
  await expectComposerFillsTheWidth(page);
  await expectSendAlignsWithTheBox(page);
});

test('transcript — an undecided question with its two buttons @ phone width', async ({
  page,
}, testInfo) => {
  // The one screen that must work under a thumb on a train: a long command and
  // two controls, on a narrow screen, with nothing pushed off the edge.
  await mockRunner(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page.getByRole('button', { name: 'allow' }).waitFor();
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo, null, BUSY_BAR);
  // The decision and the send button on one screen: the controls whose size is
  // the difference between answering from a train and waiting until you are off
  // it.
  await expectThumbTargets(page);
});

test('session list — a blocked session says so first @ phone width', async ({ page }, testInfo) => {
  await mockRunner(page);
  await page.goto('/');
  await page.getByText('waiting for you').waitFor();
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo);
});

test('the composer grows with what is being typed @ phone width', async ({ page }, testInfo) => {
  // The reason this exists: `rows="1"` meant a long instruction was written
  // through a one-line slit, on the device the console is driven from. jsdom has
  // no layout, so only a real browser can say whether the box actually grew.
  await mockRunner(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
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

  await expectNoHorizontalOverflow(page, testInfo, null, BUSY_BAR);
  await expectThumbTargets(page);
});

test('a tool result opens without widening the page @ phone width', async ({ page }, testInfo) => {
  // Unwrapped source, two thousand characters of it, on a 412px screen — the
  // widest single thing the transcript can be asked to hold.
  await mockRunner(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
  const unfold = page.getByRole('button', { name: /characters/ });
  await unfold.waitFor();
  await unfold.click();
  await page.getByText('quantiseLegCost', { exact: false }).first().waitFor();
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo, null, BUSY_BAR);
});
