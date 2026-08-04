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

/**
 * Hand the test the stream, so it decides when a message arrives.
 *
 * ⚠ **The mocked SSE body cannot answer this question.** `route.fulfill` serves
 * a complete response and closes, so every event in the fixture has landed
 * before the first paint — which is exactly the case that cannot show whether
 * the view *keeps* following. Following is about what happens to a page that is
 * already on screen, so the arrival has to be a thing the test performs.
 *
 * Replaces `EventSource` before the app loads. `follow()` in `console-api.ts`
 * assigns `onmessage` and adds a `reset` listener, and reads `data` and
 * `lastEventId` off the message — so an EventTarget with those two is the whole
 * contract.
 */
async function handControlOfTheStream(page: Page): Promise<void> {
  await page.addInitScript(() => {
    class Held extends EventTarget {
      onmessage: ((message: { data: string; lastEventId: string }) => void) | null = null;
      constructor(public url: string) {
        super();
        (window as unknown as { __stream?: Held }).__stream = this;
      }
      close(): void {
        if ((window as unknown as { __stream?: Held }).__stream === this) {
          (window as unknown as { __stream?: Held }).__stream = undefined;
        }
      }
    }
    (window as unknown as { EventSource: unknown }).EventSource = Held;
    (window as unknown as { __say: unknown }).__say = (event: unknown, seq: number) => {
      const stream = (window as unknown as { __stream?: Held }).__stream;
      if (!stream?.onmessage) return false;
      stream.onmessage({ data: JSON.stringify(event), lastEventId: String(seq) });
      return true;
    };
    // What the runner sends when it cannot resume from where the client got to:
    // a console restarted, or a session busy enough to have dropped that far out
    // of its scrollback. The client throws its transcript away and rebuilds.
    (window as unknown as { __resetStream: unknown }).__resetStream = () => {
      const stream = (window as unknown as { __stream?: Held }).__stream;
      if (!stream) return false;
      stream.dispatchEvent(new Event('reset'));
      return true;
    };
  });
}

/** Say something without waiting for the view to react — how a real answer
 *  arrives, in deltas faster than the frames that render them. */
async function mutter(page: Page, event: unknown, seq: number): Promise<boolean> {
  return page.evaluate(
    ([one, at]) => (window as unknown as { __say(e: unknown, n: number): boolean }).__say(one, at),
    [event, seq] as [unknown, number],
  );
}

/** Say something on the stream and let the view lay it out and react. */
async function say(page: Page, event: unknown, seq: number): Promise<void> {
  const delivered = await mutter(page, event, seq);
  expect(delivered, 'nothing was listening to the stream').toBe(true);
  // Two frames: `session-view.ts` follows in a `requestAnimationFrame` after the
  // entries change, so one frame is the render and the second is the reaction.
  await page.evaluate(
    () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))),
  );
}

/** How far the transcript is from its own end, in pixels. */
async function distanceFromTheEnd(page: Page): Promise<number> {
  return page.evaluate(() => {
    const box = document.querySelector('.transcript');
    if (!box) return Number.NaN;
    return box.scrollHeight - box.scrollTop - box.clientHeight;
  });
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

/** A paragraph tall enough that not following is unmistakable — well past the
 *  120px of slack `session-view.ts` allows before it calls a reader "left
 *  behind", and past a phone screen too. */
const LONG_ANSWER = Array.from(
  { length: 40 },
  (_, line) => `Line ${line + 1} of an answer long enough to move the end of the transcript.`,
).join('\n\n');

test('the transcript keeps following while the reader is at the end @ phone width', async ({
  page,
}) => {
  // The contract this pins: a reader at the end stays at the end as the session
  // talks, without touching anything. It is the normal way this page is used —
  // watching a session work — and nothing measured it, so "it stopped following"
  // had no answer but somebody's memory of it.
  await handControlOfTheStream(page);
  await mockRunner(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
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
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page.locator('.transcript').waitFor();
  await say(page, { kind: 'text', text: LONG_ANSWER }, 1);

  await page.evaluate(() => {
    const box = document.querySelector('.transcript');
    if (box) box.scrollTop = 0;
  });
  await page.evaluate(() => new Promise((done) => requestAnimationFrame(done)));
  const away = await distanceFromTheEnd(page);
  expect(away, 'the scroll back did not take').toBeGreaterThan(200);

  await say(page, { kind: 'text', text: LONG_ANSWER }, 2);
  const after = await distanceFromTheEnd(page);
  // Further from the end than before, because the transcript grew underneath a
  // reader who did not move — and emphatically not back at the bottom.
  expect(after, 'yanked the reader to the end').toBeGreaterThan(away);
});

test('the transcript follows an answer arriving in deltas @ phone width', async ({ page }) => {
  // ⚠ **The shape a real answer has**, and the one the two tests above do not
  // cover: forty small `text` events, none of them waiting for the view to
  // catch up. `follow()` reads a height inside a `requestAnimationFrame` and
  // sets `scrollTop`, and that set fires a scroll event which is what decides
  // whether the reader still counts as at the end — so deltas landing between
  // the two are the case where the page could talk itself out of following.
  await handControlOfTheStream(page);
  await mockRunner(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
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
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page.locator('.transcript').waitFor();
  await say(page, { kind: 'text', text: LONG_ANSWER }, 1);
  expect(await distanceFromTheEnd(page)).toBeLessThan(4);

  const reset = await page.evaluate(() =>
    (window as unknown as { __resetStream(): boolean }).__resetStream(),
  );
  expect(reset, 'nothing was listening for a reset').toBe(true);
  await say(page, { kind: 'joined', earlier: 1, from: 0 }, 1);
  await say(page, { kind: 'text', text: LONG_ANSWER }, 2);
  expect(await distanceFromTheEnd(page), 'left behind by a reconnect').toBeLessThan(4);
});

/** Every state the list can be in at once, in an order that is nobody's idea of
 *  correct — so the page has to be doing the sorting rather than the fixture. */
const MIXED = {
  ...STATE,
  sessions: [
    { ...STATE.sessions[1], id: 'aaaa0000-0000-4000-8000-000000000001', name: 'finished' },
    {
      ...STATE.sessions[0],
      id: 'aaaa0000-0000-4000-8000-000000000002',
      name: 'idle-one',
      busy: undefined,
      waiting: 0,
    },
    {
      ...STATE.sessions[0],
      id: 'aaaa0000-0000-4000-8000-000000000003',
      name: 'blocked',
      busy: undefined,
      waiting: 1,
    },
    { ...STATE.sessions[0], id: 'aaaa0000-0000-4000-8000-000000000004', name: 'working' },
  ],
};

/** Conversations on disk, one of them held by something the console cannot see. */
const ON_DISK = [
  {
    id: 'bbbb0000-0000-4000-8000-000000000001',
    dir: '/home/example/Code/thoth',
    modified: NEXT,
    bytes: 4_194_304,
    name: 'older',
    busy: false,
  },
  {
    id: 'bbbb0000-0000-4000-8000-000000000002',
    dir: '/home/example/Code/utterance',
    modified: LATE,
    bytes: 1_048_576,
    name: 'held-elsewhere',
    busy: true,
  },
];

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
  // ⚠ **The two dates a session has, and the card must show the second.**
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
            ...STATE.sessions[0],
            name: 'worked-all-night',
            // Picked up a day ago, answering this minute.
            started: Math.floor(Date.now() / 1000) - DAY,
            touched: Date.now() - 30_000,
          },
        ],
      },
    }),
  );
  await page.goto('/');
  await expect(page.locator('.session')).toHaveCount(1);
  const facts = await page.locator('.session .facts').first().innerText();
  expect(facts, 'the card is dating the process instead of the conversation').toContain('just now');
  expect(facts).not.toContain('24h ago');
});

test('session list — a blocked session says so first @ phone width', async ({ page }, testInfo) => {
  await mockRunner(page);
  await page.goto('/');
  await page.getByText('waiting for you').waitFor();
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo);
});

test('the transcript keeps its end while the composer grows @ phone width', async ({ page }) => {
  // ⚠ **The one thing that moves the end of the transcript without the
  // transcript changing.** The composer is `flex: 0 0 auto` above it, so every
  // line typed takes a line off the scrolling region — the reader has not
  // moved, no event has arrived, and the message being answered slides out of
  // sight. `session-view.ts` follows on `visualViewport` resize, which is the
  // soft keyboard; the box growing under a thumb is not that, and nothing else
  // asks.
  await handControlOfTheStream(page);
  await mockRunner(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
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
