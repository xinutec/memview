import { expect, test, type Page } from '@playwright/test';
// The fleet-shared harness, published as @xinutec/ui-harness (source repo
// ~/Code/ui-harness). Ships compiled JS, so it loads straight from node_modules.
import {
  expectIconFontLoaded,
  expectNoClippedText,
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

/**
 * Every icon that is a control on its own, and whether it sits in the middle of
 * it.
 *
 * ⚠ **The one failure class every other check here is blind to.** A glyph 3px
 * high in its own circle is not clipped, does not overflow, does not overlap
 * anything and is the right size to press — it is simply wrong, and the only
 * evidence is the arithmetic between two boxes. It happened the moment the
 * app-wide 48px floor was raised on a Material icon button: Material sizes those
 * 40px square with 8px of padding baked into its structural CSS, so the extra
 * 8px all went below the glyph.
 *
 * Measured against the control's own box rather than the state layer's, because
 * they are the same box when nothing is wrong and the control is the thing
 * anybody aims at. 1px of tolerance: a 24px glyph in an odd-height box is
 * legitimately half a pixel out, and `mat-icon` rounds.
 *
 * Local rather than in `@xinutec/ui-harness`, and staying local until a second
 * app wants it. Nothing about it is this app's — but `expectThumbTargets` above
 * is equally general, has caught real defects, and after all this time lives
 * here and nowhere else: measured, no other frontend in the fleet has an
 * equivalent or has missed one. The trigger for promotion is a second consumer,
 * not the observation that it could have one.
 */
async function expectIconsCentred(page: Page, slack = 1): Promise<void> {
  const skewed = await page.evaluate((tolerance) => {
    const off: { label: string; dx: number; dy: number }[] = [];
    for (const control of document.querySelectorAll('button, a[href]')) {
      const glyphs = control.querySelectorAll('mat-icon');
      // Only controls that ARE an icon. A button with an icon beside a label
      // places the pair, and neither one belongs in the middle by itself.
      if (
        glyphs.length !== 1 ||
        (control.textContent ?? '').trim() !== glyphs[0].textContent?.trim()
      )
        continue;
      const box = control.getBoundingClientRect();
      const glyph = glyphs[0].getBoundingClientRect();
      if (box.width === 0 || glyph.width === 0) continue;
      const dx = glyph.x + glyph.width / 2 - (box.x + box.width / 2);
      const dy = glyph.y + glyph.height / 2 - (box.y + box.height / 2);
      if (Math.abs(dx) <= tolerance && Math.abs(dy) <= tolerance) continue;
      off.push({
        label: glyphs[0].textContent?.trim() ?? control.className,
        dx: Math.round(dx * 10) / 10,
        dy: Math.round(dy * 10) / 10,
      });
    }
    return off;
  }, slack);
  expect(skewed, 'icons sitting off-centre in their own control').toEqual([]);
}

/**
 * Every clock in the margin, and whether it sits on the line it dates.
 *
 * ⚠ **The same blind spot `expectIconsCentred` covers, one row down.** A time
 * 6px below its own line is not clipped, does not overlap, does not overflow and
 * is perfectly legible — it just reads as a number floating near a row rather
 * than a label on it, and the only evidence is arithmetic between two boxes.
 *
 * It happened because `.at` sets `font:`, a shorthand that **resets
 * `line-height`** — so the clock stopped sharing the row's line box, and a
 * hand-tuned `top` offset covered for it at one font size while the transcript
 * has two (`.turn` is body-small, `.asked` body-medium).
 *
 * Measured against the row's own first line rather than its border box, because
 * that is the thing the clock is a label on.
 *
 * 2px of tolerance, from a measured cause rather than tuned until green: a
 * question card lays its first line out with `align-items: baseline` against a
 * monospace sibling, and the fallback face's metrics move that text by about
 * that much. The defects this exists for were 6px and 13px.
 */
async function expectClocksOnTheirLine(page: Page, slack = 2): Promise<void> {
  const adrift = await page.evaluate((tolerance) => {
    const off: { row: string; by: number }[] = [];
    for (const entry of document.querySelectorAll('.entry')) {
      const at = entry.querySelector('.at');
      if (!at) continue;
      const clock = at.getBoundingClientRect();
      // A row whose clock is hidden — `.said` and `.tool` take the time from the
      // turn line below them, so there is nothing to line up.
      if (clock.height === 0) continue;
      // The first line of TEXT, descended into rather than taken as the first
      // child — a row whose content is a card would otherwise be measured by the
      // middle of the whole box, which is not a line and not what the clock
      // labels.
      const walk = document.createTreeWalker(entry, NodeFilter.SHOW_TEXT, {
        acceptNode: (node) =>
          at.contains(node) || (node.textContent ?? '').trim() === ''
            ? NodeFilter.FILTER_REJECT
            : NodeFilter.FILTER_ACCEPT,
      });
      const first = walk.nextNode();
      if (!first) continue;
      const range = document.createRange();
      range.selectNodeContents(first);
      const line = range.getClientRects()[0];
      if (!line) continue;
      // ⚠ **Tops, not centres.** The clock is body-small beside rows that are
      // body-medium, and two line boxes of different heights cannot share a
      // centre — a centre check would have to carry a tolerance wide enough to
      // hide the defect it exists for. Where a line starts is exact at any size.
      const by = clock.top - line.top;
      if (Math.abs(by) <= tolerance) continue;
      off.push({ row: entry.className, by: Math.round(by * 10) / 10 });
    }
    return off;
  }, slack);
  expect(adrift, 'clocks sitting off the line they date').toEqual([]);
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

/**
 * A transcript ending in a question of the kind a person answers.
 *
 * Deliberately the awkward one: four options, descriptions that are whole
 * sentences, and a `multiSelect` question after it — so the page has to hold a
 * column of two-line controls AND a send button on a 412px screen. A question
 * with two one-word options would prove nothing.
 */
const QUESTION_TRANSCRIPT = [
  { kind: 'started', model: 'claude-opus-5[1m]', cwd: '/home/example/Code/memview', tools: 14 },
  { kind: 'text', text: 'Two things before I build it.', at: NEXT },
  {
    kind: 'ask',
    id: 'c9f0a1b2-0000-4000-8000-00000000000a',
    tool: 'AskUserQuestion',
    input: {
      questions: [
        {
          question: 'How far should the question UI go?',
          header: 'Scope',
          multiSelect: false,
          options: [
            {
              label: 'options only',
              description:
                'Render each option as a button and send the tap. Smallest change, and it covers the case almost every question is.',
            },
            {
              label: 'options and free text',
              description:
                'The same, plus a field whose contents ride back as the response — which is what the CLI offers under "Other".',
            },
            { label: 'full parity', description: 'Free text, multi-select and per-answer notes.' },
            { label: 'nothing', description: 'Keep asking in prose.' },
          ],
        },
        {
          question: 'Which of these should the card show?',
          header: 'On screen',
          multiSelect: true,
          options: [
            { label: 'the description', description: 'The sentence under each label.' },
            { label: 'the topic', description: 'The one-word chip above the question.' },
          ],
        },
      ],
    },
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
      // ⚠ **A fixed element is not clipped by its ancestors' overflow** — it is
      // positioned against the viewport, and climbing past it cropped the one
      // control on this page that can actually reach the stamp. Cropped to
      // `.page`, which stops above the stamp by construction, so the check
      // reported nothing however far down the button was pushed: proved by
      // moving it to `bottom: 0.25rem`, straight over the stamp, and watching
      // this pass.
      if (getComputedStyle(node).position === 'fixed') return box;
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
    // ⚠ `.add` is here because it is the one thing on this page positioned
    // against the viewport rather than laid out in the column — which is exactly
    // the way to end up on top of the stamp, and the only way this check can be
    // wrong by omission. `.start` used to be the form at the top of the page and
    // is gone; a selector naming an element that no longer exists is a check
    // quietly covering less than it reads as covering.
    for (const card of document.querySelectorAll('.page .session, .page .past, .page .add')) {
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

test('starting a session is behind one button, not in the way @ phone width', async ({
  page,
}, testInfo) => {
  // The list is what this page is opened for — a dozen conversations, scanned
  // for the one that is waiting. The form used to be the first thing on it and
  // the first thing scrolled past, costing a screenful of a phone every time.
  await mockRunner(page);
  let sent: Record<string, unknown> | undefined;
  await page.route('**/api/sessions', (r) => {
    sent = r.request().postDataJSON() as Record<string, unknown>;
    return r.fulfill({ json: STATE.sessions[0] });
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
  // Prefilled with the first repository, which is where the page used to start
  // from — one less thing to type on a phone.
  await expect(where).toHaveValue(REPOS[0]);
  // Scoped to the sheet: it is the only thing on screen that matters now, and
  // the list behind it is still in the DOM.
  await expectNoHorizontalOverflow(page, testInfo, 'mat-bottom-sheet-container');
  await expectNoClippedText(page, testInfo, 'mat-bottom-sheet-container');

  await where.fill('/home/example/Code/memview');
  await page.getByLabel('what to do (optional)').fill('check the corpus');
  await page.getByRole('button', { name: /start a session/ }).click();

  await expect
    .poll(() => sent)
    .toMatchObject({
      dir: '/home/example/Code/memview',
      prompt: 'check the corpus',
    });
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
  await expectIconsCentred(page);
  await expectClocksOnTheirLine(page);
  await expectComposerFillsTheWidth(page);
  await expectSendAlignsWithTheBox(page);
});

/** Serve the question transcript, and record what any decision sends. */
async function mockQuestion(page: Page): Promise<() => Record<string, unknown> | undefined> {
  let sent: Record<string, unknown> | undefined;
  await mockRunner(page);
  await page.route('**/api/sessions/*/events', (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: QUESTION_TRANSCRIPT.map((event) => `data: ${JSON.stringify(event)}\n\n`).join(''),
    }),
  );
  await page.route('**/api/sessions/*/decide', (r) => {
    sent = r.request().postDataJSON() as Record<string, unknown>;
    return r.fulfill({ json: STATE.sessions[0] });
  });
  return () => sent;
}

test('a question offers what was asked, not allow and refuse @ phone width', async ({
  page,
}, testInfo) => {
  // ⚠ **The defect this was built for.** The console showed a question as a
  // permission for months: the options arrived in the ask and were never
  // rendered, so the only answer it could give was an approval with no answer in
  // it — which the CLI reports as "the user did not answer the questions".
  const sent = await mockQuestion(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page.getByRole('button', { name: /options only/ }).waitFor();

  await expect(
    page.getByRole('button', { name: 'allow' }),
    'a question is not a permission',
  ).toHaveCount(0);
  // The description is the part worth reading, and the reason these are not
  // Material buttons — its label spills rather than wrapping.
  await expect(page.locator('.option .means').first()).toContainText('Smallest change');
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo, null, BUSY_BAR);
  await expectNoClippedText(page, testInfo);
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
  await page.route('**/api/sessions/*/events', (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: [
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
      ]
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page.locator('.chose').waitFor();

  await expect(page.locator('.verdict')).toHaveText('answered');
  await expect(page.locator('.chose')).toHaveText('options only · the description, the topic');
  // Nothing left to tap: the question is over.
  await expect(page.getByRole('button', { name: /options only/ })).toHaveCount(0);
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo, null, BUSY_BAR);
});

test('a typed reply is recorded as one, not as a choice @ phone width', async ({ page }) => {
  await mockRunner(page);
  await page.route('**/api/sessions/*/events', (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: [
        ...QUESTION_TRANSCRIPT,
        {
          kind: 'answered',
          id: 'c9f0a1b2-0000-4000-8000-00000000000a',
          allowed: true,
          reply: { response: 'neither — do the read-only part first' },
        },
      ]
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page.locator('.chose').waitFor();
  await expect(page.locator('.verdict')).toHaveText('replied');
  await expect(page.locator('.chose')).toHaveText('neither — do the read-only part first');
});

test('a note rides with the choice it qualifies @ phone width', async ({ page }, testInfo) => {
  // ⚠ **The difference from the reply field.** Words override the choices; a
  // note qualifies one, so both must arrive — and the options must stay live
  // while one is being written.
  const sent = await mockQuestion(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
  const options = page.getByRole('button', { name: /options only/ });
  await options.waitFor();

  await page.getByRole('button', { name: 'add a note' }).first().click();
  await page
    .getByRole('textbox', { name: /a note about How far/ })
    .fill('but keep the skip button');
  await expect(options, 'a note is not a reply and must not take the card over').toBeEnabled();
  await options.click();
  await page.getByRole('button', { name: /the description/ }).click();
  await expectNoHorizontalOverflow(page, testInfo, null, BUSY_BAR);
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
  await page.goto(`/s/${STATE.sessions[0].id}`);
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
  // ⚠ **The trap this shape exists to close.** The CLI's result builder tests
  // `response` before `answers` and reports only what it finds, so words sent
  // alongside a set of taps would throw the taps away and say nothing about it.
  // Typing therefore disables the options rather than sitting beside them.
  const sent = await mockQuestion(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
  const options = page.getByRole('button', { name: /options only/ });
  await options.waitFor();
  await expect(options).toBeEnabled();

  await page.locator('.say').first().fill('neither — do the read-only part first');
  await expect(options, 'an option that could still be tapped but would not arrive').toBeDisabled();

  const send = page.getByRole('button', { name: 'reply', exact: true });
  await expect(send, 'the button says what it will do').toBeVisible();
  await expectNoHorizontalOverflow(page, testInfo, null, BUSY_BAR);
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
  await page.goto(`/s/${STATE.sessions[0].id}`);
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
  const [first] = QUESTION_TRANSCRIPT.slice(-1);
  const alone = {
    ...first,
    input: { questions: [(first.input as { questions: unknown[] }).questions[0]] },
  };
  await page.route('**/api/sessions/*/events', (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: [QUESTION_TRANSCRIPT[0], alone]
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.route('**/api/sessions/*/decide', (r) => {
    sent = r.request().postDataJSON() as Record<string, unknown>;
    return r.fulfill({ json: STATE.sessions[0] });
  });

  await page.goto(`/s/${STATE.sessions[0].id}`);
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

test('opening a session from the list lands at the newest message @ phone width', async ({
  page,
}) => {
  // ⚠ **The path somebody actually takes**, and a different one from the tests
  // below: they drive a stream into a page that is already open, where this
  // navigates to a component that does not exist yet and is handed a whole
  // transcript at once. A resumed conversation is replayed from its last 400
  // events, so a view that opened at the top would open a hundred turns behind
  // the present — and read as the page being broken rather than as a scroll
  // position.
  await mockRunner(page);
  await page.route('**/api/sessions/*/events', (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: [
        ...TRANSCRIPT,
        // Enough afterwards to put the end of it well past a phone screen.
        ...Array.from({ length: 25 }, (_, n) => ({
          kind: 'prompt',
          text: `Follow-up ${n + 1}: check the next journey and report what moved.`,
          at: NEXT,
        })),
        { kind: 'text', text: 'THE NEWEST THING SAID' },
      ]
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
  // ⚠ **No control to press.** The seed is a page, not the conversation, and
  // reading back through a morning used to be a dozen taps on `earlier
  // messages`. Reaching the top IS the request now, so what this measures is
  // that a reader who scrolls up gets more — and keeps getting it, which is the
  // half a single fetch would not show.
  let asked = 0;
  await mockRunner(page);
  // The seed carries the byte offset it began at, and that — not the number of
  // entries — is what says the file holds more. Without it there is nothing
  // above the transcript to reach.
  await page.route('**/api/sessions/*/events', (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: [
        ...TRANSCRIPT,
        // Several screens of it, so that "at the newest message" is genuinely
        // far from the top. The seed is a page and a page is 400 events; a
        // fixture only a little taller than the screen puts the mark inside the
        // 400px of prefetch margin at every scroll position, and the check
        // below would then be asserting the margin does not exist.
        ...Array.from({ length: 40 }, (_, n) => ({
          kind: 'prompt',
          text: `Message ${n + 1} of the page that was seeded when this view opened.`,
          at: NEXT,
        })),
        { kind: 'joined', earlier: TRANSCRIPT.length + 40, from: 5000 },
      ]
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
        events: Array.from({ length: 12 }, (_, n) => ({
          kind: 'prompt',
          text: `Older page ${page_}, message ${n + 1} — something said earlier this morning.`,
          at: LATE,
        })),
      },
    });
  });
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page.locator('.transcript').waitFor();
  await expect(page.locator('.earlier')).toHaveCount(1);
  // The control it replaces is gone, not merely hidden.
  await expect(page.getByRole('button', { name: 'earlier messages' })).toHaveCount(0);

  // Nothing is asked for while the reader is at the newest message.
  expect(asked, 'fetched without being anywhere near the top').toBe(0);

  // ⚠ **One page per arrival at the top, not the whole file.** Landing a page
  // and holding the reader's place puts the mark back out of view, which is the
  // point: "starting small" would mean nothing if reaching the top once
  // unspooled a 1.4 GB transcript. So each fetch here is a fresh journey to the
  // top, which is what a reader travelling backwards through a morning does.
  const toTheTop = async () =>
    page.evaluate(() => {
      const box = document.querySelector('.transcript');
      if (box) box.scrollTop = 0;
    });

  await toTheTop();
  await expect.poll(() => asked, { timeout: 5000 }).toBe(1);
  // The dash matters: without it this also matches messages 10 to 12.
  await expect(page.getByText('Older page 1, message 1 —')).toHaveCount(1);

  await toTheTop();
  await expect.poll(() => asked, { timeout: 5000 }).toBe(2);
  await expect(page.getByText('Older page 2, message 1 —')).toHaveCount(1);

  // The third answer says `from: 0` — the start of the file — and the mark that
  // asks for more goes with it.
  await toTheTop();
  await expect.poll(() => asked, { timeout: 5000 }).toBe(3);
  await expect(page.locator('.earlier')).toHaveCount(0);

  // And no amount of scrolling asks again for a conversation that has none.
  await toTheTop();
  await page.waitForTimeout(300);
  expect(asked, 'kept asking past the start of the file').toBe(3);
});

test('a table keeps the alignment its author wrote @ phone width', async ({ page }) => {
  // ⚠ **Only a rendered page can answer this.** `marked` emits `align="right"`
  // and Angular's sanitiser keeps the attribute — both testable in jsdom, and
  // both were already true when every right-aligned column came out left. A
  // presentational attribute loses to any CSS rule, and a blanket
  // `text-align: left` on `th, td` was throwing all of it away.
  await handControlOfTheStream(page);
  await mockRunner(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page.locator('.transcript').waitFor();
  await say(
    page,
    { kind: 'text', text: '| l | m | r | n |\n|:---|:---:|---:|---|\n| 1 | 2 | 3 | 4 |\n' },
    1,
  );

  const cells = await page.evaluate(() =>
    [...document.querySelectorAll('.body table td')].map((cell) =>
      // ⚠ Chromium reports alignment that came from the `align` ATTRIBUTE as
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
  await page.goto(`/s/${STATE.sessions[0].id}`);
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

/** A session the runner has finished reading: it has a name, a model id and a
 *  permission mode, which is what the toolbar and the sheet divide between them. */
const NAMED = {
  ...STATE,
  sessions: [
    { ...STATE.sessions[0], name: 'health', mode: 'bypassPermissions' },
    STATE.sessions[1],
  ],
};

test('the toolbar says which session this is, beside what can be done to it @ phone width', async ({
  page,
}) => {
  await mockRunner(page);
  await page.route('**/api/state', (r) => r.fulfill({ json: NAMED }));
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await expect(page.locator('.bar .name')).toHaveText('health');
  // The ⋮ beside it is a glyph in a circle, and it has to be in the middle of it.
  await expectIconsCentred(page);
  // Named by what it is called rather than by a class, so the assertion is about
  // what a person — or a screen reader — can find.
  await expect(page.locator('.bar').getByRole('button', { name: /what to do with/ })).toHaveText(
    'more_vert',
  );
  // The path is what the name replaced, and the session's own header no longer
  // prints it: it is in the sheet behind the name.
  //
  // ⚠ Scoped to the header rather than to the page. A transcript is FULL of
  // paths — every `Read` argument is one — so asserting the path is nowhere on
  // screen fails against a page doing exactly what it should.
  await expect(page.locator('.head')).not.toContainText('/home/example/Code');
});

test('a session with no name yet says where it runs @ phone width', async ({ page }) => {
  // The state every session starts in: the runner has not read a name out of the
  // transcript, and the bar still has to say which conversation this is.
  await mockRunner(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await expect(page.locator('.bar .name')).toHaveText('decode');
  await expect(page.locator('.bar .name')).toHaveClass(/anonymous/);
});

test('the list says nothing about which machine it is @ phone width', async ({ page }) => {
  // "this Mac" was true of every session on the list and news to nobody. What
  // replaced it is nothing: the toolbar's right-hand side is about the session on
  // screen, and on the list there is no session on screen.
  await mockRunner(page);
  await page.goto('/');
  await page.getByText('decode').first().waitFor();
  await expect(page.locator('.bar')).not.toContainText('Mac');
  await expect(page.locator('.bar').getByRole('button')).toHaveCount(0);
});

test('a name too long for the bar gives way rather than pushing @ phone width', async ({
  page,
}, testInfo) => {
  // ⚠ **A name is arbitrary text from a transcript**, and the toolbar is a fixed
  // row holding two other things that must stay reachable. Being cut off is the
  // intended outcome; the failures are the ways it declines to be.
  //
  // ⚠ **The first version of this test passed against the defect.** It asserted
  // no horizontal overflow and no undersized control, and both were true while
  // the label spilled 69px out of its own button and painted over the `console`
  // link: an inline span cannot be ellipsised, so `overflow: hidden` on it did
  // nothing, and Material's label wrapper would not shrink. Nothing was off the
  // screen, so nothing failed. Hence the assertions below are about the LABEL
  // and its box, not about the page.
  await mockRunner(page);
  await page.route('**/api/state', (r) =>
    r.fulfill({
      json: {
        ...STATE,
        sessions: [
          { ...STATE.sessions[0], name: 'health-sync-backend-decode-matcher-gate-quantiser' },
        ],
      },
    }),
  );
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page.locator('.bar .name').waitFor();
  await expectNoHorizontalOverflow(page, testInfo, null, BUSY_BAR);
  await expectThumbTargets(page);

  const bar = await page.evaluate(() => {
    // `HTMLElement`, not `Element`: `scrollWidth`/`clientWidth` are what this
    // measures, and they are the whole assertion.
    const name = document.querySelector<HTMLElement>('.bar .name')!;
    const named = document.querySelector('.bar .named')!.getBoundingClientRect();
    const menu = document
      .querySelector('.bar button[aria-haspopup="menu"]')!
      .getBoundingClientRect();
    const home = document.querySelector('.bar .home')!.getBoundingClientRect();
    return {
      wanted: name.scrollWidth,
      given: name.clientWidth,
      label: name.getBoundingClientRect(),
      named,
      menuLeft: menu.left,
      menuRight: menu.right,
      homeRight: home.right,
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
  // And it was cut BY ITS OWN BUTTON, rather than spilling out of it over the
  // link to its left.
  expect(bar.label.left, 'the label starts left of its own button').toBeGreaterThanOrEqual(
    bar.named.left,
  );
  expect(bar.label.right, 'the label runs past its own button').toBeLessThanOrEqual(
    bar.named.right,
  );
  expect(bar.label.left, 'the name is painting over the home link').toBeGreaterThanOrEqual(
    bar.homeRight,
  );
  // The overflow button keeps its place at the end of the row.
  expect(bar.named.right, 'the name is under the overflow button').toBeLessThanOrEqual(
    bar.menuLeft,
  );
  expect(bar.menuRight, 'the overflow button was pushed off the edge').toBeLessThanOrEqual(
    bar.page,
  );
});

test('the session is still named after scrolling to the end @ phone width', async ({ page }) => {
  // Which conversation you are in is the one fact worth having on screen at all
  // times — the console drives a dozen at once and they differ by name. Both the
  // toolbar and the facts row sit outside the scrolling region (`.transcript` is
  // the only thing on this page that scrolls), so this measures that they do.
  await mockRunner(page);
  await page.route('**/api/state', (r) => r.fulfill({ json: NAMED }));
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page.getByText('verified_cli').first().waitFor();
  const tops = () =>
    page.evaluate(() => ({
      name: document.querySelector('.bar .name')!.getBoundingClientRect().top,
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
  await expect(page.locator('.bar .name')).toBeInViewport();
});

test('the details sheet holds what the page has no room for @ phone width', async ({
  page,
}, testInfo) => {
  // ⚠ **The first overlay this suite has ever measured.** The console has had a
  // menu in the toolbar since it was written and no check has ever opened it: an
  // overlay renders outside the component tree, so nothing on the page below is
  // evidence about it. A sheet of paths and identifiers is exactly the content
  // that pushes a 412px screen sideways.
  await mockRunner(page);
  await page.route('**/api/state', (r) => r.fulfill({ json: NAMED }));
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page.locator('.bar .named').click();
  const sheet = page.locator('.session-sheet');
  await sheet.waitFor();

  const said = await sheet.innerText();
  // The path, which nothing on the screen shows any more.
  expect(said).toContain(STATE.sessions[0].dir);
  // The session id, which nothing in the console showed anywhere.
  expect(said).toContain(STATE.sessions[0].id);
  // The model as it is shipped, not the one word the facts row has room for.
  expect(said).toContain('claude-opus-5[1m]');
  // The permission mode in words. The facts row has only its icon, and a
  // `title=` tooltip on a phone is text nobody can reach.
  expect(said).toContain('Bypass Permissions');

  await expectNoTextOverlaps(page, testInfo, '.session-sheet');
  await expectNoHorizontalOverflow(page, testInfo, '.session-sheet');
  await expectNoClippedText(page, testInfo, '.session-sheet');
});

test('leaving a session leaves its name behind @ phone width', async ({ page }) => {
  // ⚠ **The toolbar kept the session you had just left.** `ngOnDestroy` clears
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
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page.locator('.transcript').waitFor();
  // Away before the runner has answered.
  await page.locator('.home').click();
  await page.locator('.session').first().waitFor();
  answer?.();
  // Long enough for the held response to land and be acted on.
  await page.waitForTimeout(300);
  await expect(
    page.locator('.bar .name'),
    'the list is titled with the session just left',
  ).toHaveCount(0);
  await expect(page.locator('.bar').getByRole('button')).toHaveCount(0);
});
