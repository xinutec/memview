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
 * Let every transform that is still moving finish, before anything measures a
 * box.
 *
 * ⚠ **A box measured mid-animation is not the box the layout claims.** Material
 * opens its overlays by scaling them: `_mat-menu-enter` runs a panel from
 * `scale(0.8)` to `scale(1)` over 120ms, and `getBoundingClientRect` maps
 * through that, so every 48px menu item inside is genuinely 38px early in the
 * window and 47.99px in the last frame of it. Both read as a control too small
 * to hit, and neither is: measured 2026-08-12, `expectThumbTargets` failed 6/6
 * on an open menu with the CPU throttled 8× and passes 6/6 with this wait
 * (memview #735).
 *
 * ⚠ **This is not "wait for the overlay to be ready".** Nothing running means it
 * returns at once, and an overlay whose contents are still on their way has
 * nothing running yet — so a check that measures those contents must wait for
 * them to EXIST first and use this only to stop them moving. See [[openParse]],
 * where using it alone read `.raw` as `null` on a page about to be correct.
 *
 * ⚠ **Finite animations only, and this is not a detail.** A busy session keeps
 * `mat-progress-bar` cycling forever, so waiting for `getAnimations()` to drain
 * would never return on the page these checks are mostly pointed at. Bounded as
 * well, because an animation that is never going to finish should fail the
 * assertion that follows rather than the 90s test timeout.
 */
async function settleTransforms(page: Page, bound = 2_000): Promise<void> {
  await page.evaluate(async (ms) => {
    const moving = document.getAnimations().filter((a) => {
      if (a.effect?.getTiming().iterations === Infinity) return false;
      const frames = a.effect instanceof KeyframeEffect ? a.effect.getKeyframes() : [];
      return frames.some((frame) => 'transform' in frame || 'scale' in frame);
    });
    if (moving.length === 0) return;
    // `finished` rejects on a cancelled animation — an overlay torn down while
    // it was still arriving is settled by any reading that matters here.
    const done = Promise.all(moving.map((a) => a.finished.catch(() => undefined)));
    await Promise.race([done, new Promise((r) => setTimeout(r, ms))]);
  }, bound);
}

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
  await settleTransforms(page);
  const small = await page.evaluate((least) => {
    const missed: { label: string; width: number; height: number; scaledBy?: string }[] = [];
    for (const control of document.querySelectorAll('button, a[href]')) {
      const box = control.getBoundingClientRect();
      // Not rendered at all — a disabled control in a collapsed branch.
      if (box.width === 0 && box.height === 0) continue;
      if (box.height >= least && box.width >= least) continue;
      // ⚠ **Reported to the hundredth, deliberately.** Rounding these to whole
      // pixels cost two days of #735: a row that missed by 0.004px was reported
      // as `height: 48`, so the number in the failure said the row was the size
      // the assertion had just refused. Whatever is left of that flake, its
      // evidence now arrives with the digits the argument is about.
      const round = (n: number) => Math.round(n * 100) / 100;
      // And by what, if anything, it was being scaled while it was measured —
      // the one cause this check cannot tell apart from a control that is
      // simply too small.
      let scaled: string | undefined;
      for (let el: Element | null = control; el && !scaled; el = el.parentElement) {
        const t = getComputedStyle(el).transform;
        if (t && t !== 'none') scaled = `${el.tagName.toLowerCase()} ${t}`;
      }
      missed.push({
        label: (control.textContent ?? '').trim().slice(0, 40) || control.className,
        width: round(box.width),
        height: round(box.height),
        ...(scaled === undefined ? {} : { scaledBy: scaled }),
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
  // The same reason as `expectThumbTargets`: two boxes compared while an
  // overlay is still scaling are two boxes nobody has looked at yet.
  await settleTransforms(page);
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
      // ⚠ **Without this the header's mode icon does not exist in the harness.**
      // Both the card and the session header draw it only for a session whose
      // mode the runner has read (`@if (modeIcon(); as icon)`), so a fixture
      // with none put every control check — thumb targets, icon centring,
      // overlap, overflow — on a row missing an element. It cost a real
      // regression: making that glyph a button inherited the app-wide 3rem
      // floor, the header went 19px → 40px, and the full gate passed. It was
      // caught by looking at the render.
      //
      // `acceptEdits` rather than the loudest mode: `NAMED` below already
      // carries `bypassPermissions`, so between them the quiet and the shouting
      // variants are both on screen somewhere.
      mode: 'acceptEdits',
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
  // Who is holding what, keyed by id the way the runner sends it — every holder
  // counted in one sweep, so the row and the ⋮ menu both read the same two
  // numbers without opening anything. The second session keeps no list, which is
  // the ordinary case and the reason a row can say nothing rather than `0/0`.
  // See `console/src/tasks.rs`.
  tasks: {
    sessions: { '6f7c2f11-0000-4000-8000-000000000001': { open: 2, total: 3 } },
    elsewhere: [],
  },
  // A reading in the state it usually arrives in: hours old, its short window
  // long since turned over. See `console/src/usage.rs`.
  usage: {
    host: 'mac-mini',
    age_ms: 4 * 3_600_000,
    five_hour: { pct: 28 },
    seven_day: { pct: 66, resets_in_ms: 54 * 3_600_000 },
  },
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
  // A call whose result was never written, and the boundary that reveals it. The
  // process that would have recorded the answer died first, so the row carries
  // no verdict and no clock — a third state beside the tick and the cross, and
  // one a 412px row has to hold without pushing the tool's own argument out.
  {
    kind: 'tool',
    id: 'toolu_00',
    name: 'Bash',
    input: { command: 'nix develop -c home-manager switch --flake .#pippijn' },
    at: LATE,
  },
  // ⚠ **Carries an `at`, and that is the regression case for memview#725.** A
  // real `Joined` always does — the runner pushes it through `push()`, which
  // stamps `now()` — so a note row without one tests a shape production never
  // sends. With one, this row is drawn with a clock in the margin, which is what
  // caught an input's styling leaking onto it through an unscoped `.note`.
  { kind: 'joined', earlier: 1, from: 0, at: LATE },
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
/**
 * Open the folded run of tool calls, so the calls themselves are on the page.
 *
 * ⚠ **A run of two or more calls is folded by default** — see `blocks()` in
 * transcript.ts — so a check about what a tool row looks like has to open it
 * first. That the fixture folds at all is the subject of its own test below.
 */
async function openTools(page: Page): Promise<void> {
  await page.locator('.entry.tools .run').first().click();
}

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
  // two controls beside it.
  expect(width!.composer).toBe(width!.page);
  // ⚠ **Three fifths, and it was seven tenths.** The row carried one button —
  // send — until the image picker joined it, and each is held to the 48px thumb
  // floor: 412px of screen less 32 of padding, 96 of buttons and 16 of gaps
  // leaves 268. Lowered deliberately rather than quietly, and no further: the
  // point of the check is that the box is the row, and a third control on it
  // would take it below this.
  expect(width!.field).toBeGreaterThan(width!.page * 0.6);
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
 * Words that look like one line sit on one line.
 *
 * ⚠ **The failure class none of the others can see.** Text a few pixels off its
 * neighbours is not clipped, not overlapping, not overflowing and not small — it
 * is simply wrong, and the only symptom is a row that reads as ragged. Both
 * defects this has caught were found by eye on the phone, not by a check.
 *
 * **Optical middles, not baselines — and this was baselines first.** A baseline
 * is the right rule for words of one size; a row mixing a 14px name with an 11px
 * pill is a different problem, because the smaller text's glyphs then sit LOWER
 * than the larger text's even though the two share a line perfectly. Measured on
 * the real page while it was baseline-aligned: the name's glyphs centred on 10.0
 * and everything beside it on 11.1, and the pill — a filled shape, not words —
 * hung 2px lower still. Reported as "almost right, and I cannot say why", which
 * is exactly what a rule that is nearly the right rule produces.
 *
 * So each word's middle is taken as half a cap height above its own baseline,
 * and those are what have to agree. The baseline is measured rather than
 * estimated: a zero-height inline-block with `vertical-align: baseline` has its
 * bottom edge ON the line box's baseline, so the probe reports what the browser
 * actually did. The cap height is Roboto's, which is the only face this app sets.
 *
 * Words are grouped into bands by their boxes overlapping vertically, which is
 * what makes this survive a head that wraps — a second line is its own band, not
 * a violation. Icon glyphs are excluded: an icon font's middle is its own
 * business, and nudging a glyph to sit right beside digits is the fix, not the
 * defect.
 */
async function expectOneLine(page: Page, rowSel: string, tol = 1): Promise<void> {
  const ragged = await page.evaluate(
    ([sel, tolerance]) => {
      const bad: string[] = [];
      for (const row of document.querySelectorAll(sel)) {
        const words: { text: string; top: number; bottom: number; base: number }[] = [];
        const walk = document.createTreeWalker(row, NodeFilter.SHOW_TEXT);
        for (let node = walk.nextNode(); node; node = walk.nextNode()) {
          const text = node.textContent?.trim();
          const parent = node.parentElement;
          if (!text || !parent) continue;
          if (parent.closest('mat-icon, .material-icons, .material-symbols-outlined')) continue;
          const style = getComputedStyle(parent);
          if (style.visibility === 'hidden' || style.display === 'none') continue;

          const range = document.createRange();
          range.selectNodeContents(node);
          const box = range.getBoundingClientRect();

          // The text is moved into a holder so the probe shares its line even
          // when the parent is a flex container, where a bare probe would become
          // a flex item of its own and measure nothing.
          const holder = document.createElement('span');
          const probe = document.createElement('span');
          probe.style.cssText = 'display:inline-block;width:0;height:0;vertical-align:baseline';
          node.parentNode?.insertBefore(holder, node);
          holder.append(node, probe);
          const baseline = probe.getBoundingClientRect().bottom;
          holder.parentNode?.insertBefore(node, holder);
          holder.remove();

          // Roboto's cap height, which is what the eye lines a row up by — half
          // of it above the baseline is where a word's middle is.
          const cap = 0.711 * Number.parseFloat(style.fontSize);
          const base = baseline - cap / 2;

          words.push({ text: text.slice(0, 20), top: box.top, bottom: box.bottom, base });
        }
        // One band per visual line: a word joins the band its box overlaps.
        for (const word of words.sort((a, b) => a.top - b.top)) {
          const band = words.filter((other) => other.top < word.bottom && other.bottom > word.top);
          const spread =
            Math.max(...band.map((w) => w.base)) - Math.min(...band.map((w) => w.base));
          if (spread > tolerance) {
            bad.push(
              `${band.map((w) => `"${w.text}" at ${w.base.toFixed(1)}`).join(', ')} — ${spread.toFixed(1)}px apart`,
            );
            break;
          }
        }
      }
      return bad;
    },
    [rowSel, tol] as [string, number],
  );
  expect(ragged, `words on one line do not share a middle in ${rowSel}`).toEqual([]);
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
      // ⚠ **The boundary first, exactly as the runner sends it.** Every real
      // connection carries `caught-up` once its backlog is flushed, and the page
      // treats everything before it as replayed history that may not speak for
      // the present. A mock that skipped it would have the page ignoring its own
      // fixtures — which is not a failure any assertion here would explain.
      const stated = stream as unknown as { __live?: boolean };
      if (!stated.__live) {
        stated.__live = true;
        stream.dispatchEvent(new Event('caught-up'));
      }
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

/**
 * A finger on the transcript: down, dragged `by` CSS pixels, then whatever
 * `during` does, then up.
 *
 * ⚠ **Trusted touch events through CDP, not `dispatchEvent`.** A synthetic
 * `TouchEvent` built in the page reaches our listeners and moves nothing —
 * untrusted events never reach the compositor — so the engine would be asked
 * about a hold during which the view never actually moved, which is the one
 * thing this is here to reproduce. Same argument as the wheel below.
 *
 * Positive `by` drags the finger DOWN the screen, which walks the transcript
 * BACK: `scrollTop` falls. Chromium eats its own gesture slop first, so the
 * finger always travels further than the view does — how much further is the
 * platform's business, which is why the tests assert on the view's movement
 * rather than on this number.
 */
async function thumb(
  page: Page,
  by: number,
  during?: () => Promise<void>,
): Promise<void> {
  const box = (await page.locator('.transcript').boundingBox())!;
  const x = Math.round(box.x + box.width / 2);
  const y = Math.round(box.y + box.height / 2);
  const cdp = await page.context().newCDPSession(page);
  const finger = { id: 1, radiusX: 8, radiusY: 8, force: 1 };
  await cdp.send('Input.dispatchTouchEvent', {
    type: 'touchStart',
    touchPoints: [{ ...finger, x, y }],
  });
  // ⚠ **Paced like a hand, not dispatched in a burst.** Sent back to back, the
  // moves read as a flick: the compositor takes the gesture, the page stops
  // receiving the touch stream after about five moves, and `touchend` never
  // arrives at all — leaving the transcript held for ever, which looks exactly
  // like the defect under test. `synthesizeScrollGesture` is not the way out
  // either; it does not drive an inner scroller.
  for (let step = 1; step <= 12; step++) {
    await cdp.send('Input.dispatchTouchEvent', {
      type: 'touchMove',
      touchPoints: [{ ...finger, x, y: y + Math.round((by * step) / 12) }],
    });
    await page.waitForTimeout(16);
  }
  // ⚠ **Still before it lifts, or this is a flick and not a thumb.** Releasing
  // mid-motion leaves a fling that keeps scrolling after the finger has gone, and
  // the engine then unpins for the best of reasons — the reader really is
  // travelling away. Measured: without this the view reads as at the end the
  // instant the touch ends, and 2,500px from it a moment later. A resting thumb
  // stops before it lifts, so its velocity at release is zero.
  for (let still = 0; still < 3; still++) {
    await cdp.send('Input.dispatchTouchEvent', {
      type: 'touchMove',
      touchPoints: [{ ...finger, x, y: y + by }],
    });
    await page.waitForTimeout(60);
  }
  if (during) await during();
  await cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
  await page.evaluate(
    () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))),
  );
  await cdp.detach();
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
  // ⚠ **Prefilled with where sessions ACTUALLY run**, not with the first
  // repository alphabetically. The old default was a real directory nothing had
  // ever been started in, which is the kind of wrong that looks deliberate — see
  // [[SessionsView.commonest]]. Here that is the busiest session's own
  // directory, which both mocked sessions share.
  await expect(where).toHaveValue(STATE.sessions[0].dir);
  // ⚠ **Opening the sheet must not open a list.** The field opens on `~/Code`,
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
  await expectNoHorizontalOverflow(page, testInfo, 'mat-bottom-sheet-container');
  await expectNoClippedText(page, testInfo, 'mat-bottom-sheet-container');

  // The opening-instruction field is gone: it duplicated the composer this sheet
  // navigates straight to, and did exactly the same thing.
  await expect(page.getByLabel('what to do (optional)')).toHaveCount(0);

  await where.fill('/home/example/Code/memview');
  await page.getByRole('button', { name: /start a session/ }).click();

  await expect.poll(() => sent).toMatchObject({ dir: '/home/example/Code/memview', prompt: '' });
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
  await openTools(page);
  await page.getByText('verified_cli').first().waitFor();
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo, null, BUSY_BAR);
  await expectNoPinnedOverlap(page);
  await expectIconsCentred(page);
  await expectClocksOnTheirLine(page);
  await expectComposerFillsTheWidth(page);
  await expectSendAlignsWithTheBox(page);
});

/**
 * Landmarks as the runner sends them, and shaped like the hard cases.
 *
 * ⚠ **The long one is not padding.** A first line is often a pasted path or a
 * command with no spaces in it, which is exactly what breaks a single-line row:
 * `overflow-wrap: anywhere` collapses the column to its longest word
 * (DL-CSS-ANYWHERE) and a missing `min-width: 0` pushes the icon off the screen.
 * Two days, so the grouping is drawn rather than assumed.
 */
/**
 * A landmark's moment, as a wall-clock time on a day counted back from today.
 *
 * ⚠ **Anchored to the run's own clock, and it has to be.** `byDay` names a day
 * `Today` or `Yesterday` by comparing `toDateString()` against `Date.now()`, so a
 * fixture pinned to absolute dates passes on the day it is written and fails
 * every day after — which is what happened: `Date.UTC(2026, 7, 11)` and
 * `(2026, 7, 12)` read as Yesterday and Today on 2026-08-12 and as neither on the
 * 13th. Local rather than UTC for the same reason `byDay` is local: the two must
 * agree about which calendar day a moment falls on, or the test asserts against a
 * grouping the app would never produce.
 */
function daysBack(days: number, hour: number, minute: number): number {
  const when = new Date(Date.now() - days * 86_400_000);
  when.setHours(hour, minute, 0, 0);
  return when.getTime();
}

const LANDMARKS = [
  {
    at: 1024,
    when: daysBack(1, 9, 14),
    kind: 'prompt',
    text: 'run the decoder against /home/example/Code/health/packages/health-sync-backend/src/decode/fixtures/2026-07-31-overnight.json and say what it makes of the gaps',
  },
  { at: 4096, when: daysBack(1, 11, 2), kind: 'command', text: 'compact' },
  { at: 8192, when: daysBack(1, 11, 2), kind: 'compacted', text: '' },
  { at: 16384, when: daysBack(0, 8, 30), kind: 'shown', text: 'screenshot-3.png' },
  { at: 32768, when: daysBack(0, 9, 5), kind: 'prompt', text: 'that is the one, thanks' },
];

test('go to — a long conversation is reachable by landmark @ phone width', async ({
  page,
}, testInfo) => {
  // ⚠ **The screen this exists for.** A transcript here runs to hundreds of
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
      json: { events: [{ kind: 'text', text: 'what was said back then' }], from: 0 },
    }),
  );
  await page.goto(`/s/${STATE.sessions[0].id}`);
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
  await expectNoHorizontalOverflow(page, testInfo, 'mat-bottom-sheet-container');
  await expectNoClippedText(page, testInfo, "mat-bottom-sheet-container");
  // Scoped to the sheet: it sits OVER the transcript by design, so an unscoped
  // check reports the conversation behind it overlapping every row.
  await expectNoTextOverlaps(page, testInfo, 'mat-bottom-sheet-container');
  // Every row here is pressable, unlike the task sheet where most are not — so
  // the whole row is the target and it may not be shaved.
  await expectThumbTargets(page);

  // ⚠ **And the state a jump leaves behind, which is the part that can mislead.**
  // Detached, the transcript does not grow and the header's "working" is a
  // reading from before — so the banner is the only thing on screen telling the
  // truth about what is being looked at.
  await sheet.getByText('that is the one, thanks').click();
  const adrift = page.locator('.deaf.adrift');
  await expect(adrift, 'nothing says this is not the live conversation').toBeVisible();
  await expect(adrift.getByText('Looking back.')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Back to now' })).toBeVisible();
  await expectNoHorizontalOverflow(page, testInfo);
  await expectNoTextOverlaps(page, testInfo);
  await expectThumbTargets(page);
});

/**
 * A real 2×4 PNG, on disk beside this spec.
 *
 * ⚠ **A file rather than bytes in the source, and not for tidiness.** PNG bytes
 * written by hand do not decode, and the failure is quiet in exactly the wrong
 * way: `setInputFiles` succeeds, the change event fires, and `createImageBitmap`
 * refuses with "the source image could not be decoded" — which the page reports
 * on the composer and a test sees only as a strip that never appeared. Two
 * fixtures were fabricated before this was a real picture made by `sips`.
 *
 * A path, not a Buffer: these specs are type-checked without Node's types (see
 * tsconfig.e2e.json), and Playwright resolves a relative path against the
 * config's directory.
 */
function tinyPng(): string {
  // Anchored to the project's own test directory rather than to the process's
  // working directory, which is the repo's frontend/ when the gate runs this and
  // something else when it is run by hand. `test.info()` is Playwright's own and
  // needs no Node types.
  return `${test.info().project.testDir}/fixtures/tiny.png`;
}

test('a picture waits to be sent with what is said about it @ phone width', async ({
  page,
}, testInfo) => {
  // ⚠ **The phone is where the screen being talked about is.** A layout that
  // settles wrongly, a chart that reads oddly, a thing on a desk — all of it was
  // describable and not showable until this. The picture is scaled in the page
  // (see `picture.ts`) and sent as an `image` block on the session's stdin, which
  // the CLI forwards and the model reads — measured against 2.1.221 first.
  let sent: Record<string, unknown> | undefined;
  await mockRunner(page);
  await page.route('**/api/sessions/*/image', (r) => {
    sent = r.request().postDataJSON() as Record<string, unknown>;
    return r.fulfill({ json: STATE.sessions[0] });
  });
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page.locator('.composer').waitFor();

  // A real 2×4 PNG, chosen the way the picker hands one over.
  await page.locator('.picker').setInputFiles(tinyPng());

  // Held, not sent: the words about a screenshot are usually the point of it, so
  // it waits in a strip above the box until there is something to say.
  const chosen = page.locator('.chosen');
  await chosen.waitFor();
  await expect(chosen.locator('.thumb')).toBeVisible();
  await expect(chosen.locator('.about')).toContainText('2×4');
  expect(sent, 'nothing left the phone on choosing').toBeUndefined();

  // ⚠ The composer holds a thumbnail, a size, a discard button, the box and
  // send — the fullest this row ever gets, and the phone is 412px wide.
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo, null, BUSY_BAR);
  await expectNoPinnedOverlap(page);
  await expectThumbTargets(page);

  await page.locator('.composer textarea').fill('what is wrong with this?');
  await page.locator('.composer .send').click();
  await expect.poll(() => sent?.['media_type']).toBe('image/png');
  expect(sent?.['text'], 'the words travel with the picture').toBe('what is wrong with this?');
  expect(String(sent?.['data']), 'bare base64, as the API defines it').not.toContain('data:');

  // And the strip goes with the message, so the next one does not carry it again.
  await expect(page.locator('.chosen')).toHaveCount(0);
});

test('a command waiting for the turn says so, and can be taken back @ phone width', async ({
  page,
}, testInfo) => {
  // ⚠ **Measured 2026-08-08 against CLI 2.1.221/226.** A slash command written
  // to a working session is not run: the CLI parks it as a `queued_command` with
  // `commandMode: "prompt"` and hands it to the MODEL as words. `/rename` sent
  // from the phone got "Noted the rename (CLI-side, nothing for me to do)" and
  // no name was ever written. The runner holds it now; this is the screen saying
  // so, which is the half that makes it not a second silent thing.
  await mockRunner(page);
  const id = STATE.sessions[0].id;
  const working = { ...STATE.sessions[0], working: true, held: ['/compact'] };
  await page.route('**/api/state', (r) =>
    r.fulfill({ json: { ...STATE, sessions: [working, STATE.sessions[1]] } }),
  );
  let cancelled: Record<string, unknown> | undefined;
  await page.route(`**/api/sessions/${id}/unhold`, (r) => {
    cancelled = r.request().postDataJSON() as Record<string, unknown>;
    return r.fulfill({ json: { ...working, held: [] } });
  });
  await page.goto(`/s/${id}`);

  const chip = page.locator('.waiting');
  await chip.waitFor();
  await expect(chip).toContainText('/compact');
  await expect(chip, 'the chip does not say when it will run').toContainText(
    'runs when this turn ends',
  );
  // The fullest this strip gets is a long command beside its own button, on a
  // phone 412px wide.
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo, null, BUSY_BAR);
  await expectThumbTargets(page);

  await chip.getByRole('button', { name: /do not run/ }).click();
  await expect.poll(() => cancelled?.['text']).toBe('/compact');
  // Gone because the runner said it is gone, not because the button was pressed.
  await expect(page.locator('.waiting')).toHaveCount(0);
});

test('what is being written survives leaving the conversation @ phone width', async ({ page }) => {
  // Reported 2026-08-06: typed words and a chosen picture were lost on going up
  // to the list and coming back. The picture is the expensive half — it cost a
  // scale of a phone photograph — and the words are the half nobody wants to
  // type twice on a phone.
  await mockRunner(page);
  const first = STATE.sessions[0].id;
  const second = STATE.sessions[1].id;
  await page.goto(`/s/${first}`);
  await page.locator('.composer').waitFor();
  await page.locator('.picker').setInputFiles(tinyPng());
  await page.locator('.chosen').waitFor();
  await page.locator('.composer textarea').fill('half a thought about this');

  // Up to the list, which destroys the view — the actual reported action, and
  // not `goto`, which would reload the page and prove something else.
  await page.locator('.leave').click();
  await expect(page.locator('.composer')).toHaveCount(0);

  // ⚠ Through the OTHER conversation on the way back, because a draft that was
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
  // ⚠ **Visible is not drawn.** A held picture's preview is an object URL, which
  // belongs to the document that made it and is dead in the next one — a revived
  // draft carrying one would show a broken image that still passes every
  // assertion above. `naturalWidth` is the browser saying it decoded the bytes.
  await expect
    .poll(() =>
      page.locator('.chosen .thumb').evaluate((img) => (img as HTMLImageElement).naturalWidth),
    )
    .toBe(2);
});

test('a picture that was sent is on the screen, not a path to it @ phone width', async ({
  page,
}, testInfo) => {
  // ⚠ **The half of the feature that was missing.** A sent picture reached the
  // model and left the person who took it with a sentence about a file path —
  // the one party to the conversation who could not see it. The runner reads the
  // image block back out of the transcript, keeps the note out of the words, and
  // says which file; the page fetches it.
  let asked = '';
  await mockRunner(page);
  await page.route('**/api/sessions/*/events', (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: [
        { kind: 'started', model: 'claude-opus-5[1m]', cwd: '/home/example/Code', tools: 14 },
        { kind: 'shown', name: '2026-08-05-184700Z.png', at: NEXT },
        { kind: 'prompt', text: 'what is wrong with this?', at: NEXT },
      ]
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.route('**/api/sessions/*/images/*', (r) => {
    asked = r.request().url();
    return r.fulfill({ path: tinyPng(), contentType: 'image/png' });
  });
  await page.goto(`/s/${STATE.sessions[0].id}`);

  const picture = page.locator('.picture img');
  await picture.waitFor();
  expect(asked, 'asked for by name, under the session it was sent to').toContain(
    '2026-08-05-184700Z.png',
  );
  // Decoded, not merely requested: a broken image is a visible element with no
  // pixels in it, which every assertion about visibility would still pass.
  expect(await picture.evaluate((img: HTMLImageElement) => img.naturalWidth)).toBe(2);
  // The words came with it, and the note about where the file was kept did not:
  // that sentence is addressed to the session, and the reader is looking at the
  // thing it describes.
  await expect(page.locator('.entry.asked')).toContainText('what is wrong with this?');
  await expect(page.locator('.entry.asked')).not.toContainText('.console/images');

  // Bounded until it is asked to be otherwise, or a screenshot is a screenful of
  // scrolling between two sentences.
  const closed = await picture.boundingBox();
  await page.locator('.picture').click();
  await expect(picture).toHaveClass(/full/);
  const open = await picture.boundingBox();
  expect(open?.width ?? 0).toBeGreaterThan(closed?.width ?? 0);

  await expectNoHorizontalOverflow(page, testInfo, null, BUSY_BAR);
});

test('a finger on the transcript stops it being pulled to the end @ phone width', async ({
  page,
}) => {
  // ⚠ **Reported from the phone.** A session writing its answer pulled the view
  // to the end on every delta, including while the reader had a thumb on the
  // glass reading the sentence as it arrived — they had not scrolled, so they
  // were still pinned, and being pinned is exactly what moved the view.
  //
  // The rules themselves are unit-tested in `following.spec.ts`, which is where
  // the numbers live. What only a browser can answer is whether the touch ever
  // reaches them: the handlers are template bindings, and a binding that is not
  // there fails silently and looks like the old behaviour.
  await mockRunner(page);
  await page.route('**/api/sessions/*/events', (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: [
        { kind: 'started', model: 'claude-opus-5[1m]', cwd: '/home/example/Code', tools: 14 },
        ...Array.from({ length: 60 }, (_, n) => ({
          kind: 'text',
          text: `paragraph ${n} of an answer long enough to scroll.\n\n`,
          at: NEXT,
        })),
      ]
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.goto(`/s/${STATE.sessions[0].id}`);
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

  // ⚠ **Wait for the resize to have been acted on, then assert.** The claim here
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

test('a picture can be put down again without being sent @ phone width', async ({ page }) => {
  // The discard is not decoration: the picker is a gallery on a phone and the
  // wrong screenshot is one tap away from the right one.
  await mockRunner(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page.locator('.composer').waitFor();
  await page.locator('.picker').setInputFiles(tinyPng());
  await page.locator('.chosen').waitFor();

  await page.getByRole('button', { name: 'do not send this image' }).click();

  await expect(page.locator('.chosen')).toHaveCount(0);
  // Send goes back to needing words, which is what it needed before there was a
  // picture to send on its own.
  await expect(page.locator('.composer .send')).toBeDisabled();
});

/**
 * A run of calls with a permission question standing on the newest one.
 *
 * ⚠ **The shape the defect lived in** (memview#86): the CLI announces the call
 * and then asks about it, carrying the same `tool_use` id on both. Two calls
 * before it, so the fold has something to fold — the card used to sit between
 * them and break every run.
 */
const DECIDING = [
  { kind: 'started', model: 'claude-opus-5[1m]', cwd: '/home/example/Code/memview', tools: 14 },
  { kind: 'tool', id: 'toolu_a1', name: 'Bash', input: { command: 'git status' }, at: NEXT },
  { kind: 'tool_result', id: 'toolu_a1', ok: true, detail: 'nothing to commit' },
  { kind: 'tool', id: 'toolu_a2', name: 'Bash', input: { command: 'git diff --stat' }, at: NEXT },
  { kind: 'tool_result', id: 'toolu_a2', ok: true, detail: '2 files changed' },
  {
    kind: 'tool',
    id: 'toolu_a3',
    name: 'Write',
    input: { file_path: '/home/example/Code/memview/notes.md', content: 'hi\n' },
    at: NEXT,
  },
  {
    kind: 'ask',
    id: 'c8471a53-2b00-4a5e-a8d3-610f6d8c6b07',
    call: 'toolu_a3',
    tool: 'Write',
    title: 'Claude wants to write /home/example/Code/memview/notes.md',
    input: { file_path: '/home/example/Code/memview/notes.md', content: 'hi\n' },
    at: NEXT,
  },
];

test('a call waiting to be allowed is one widget, not two @ phone width', async ({
  page,
}, testInfo) => {
  // ⚠ **Reported 2026-08-06, diagnosed 2026-08-11 by driving a real session.**
  // The CLI emits `tool toolu_…` and then `ask …` about the same call, and the
  // console drew both: a tool row AND a permission card for one Write. The card
  // also sat between the calls either side of it, so a sequence of decided calls
  // could never fold into one list.
  await mockRunner(page);
  await page.route('**/api/sessions/*/events', (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: DECIDING.map((event) => `data: ${JSON.stringify(event)}\n\n`).join(''),
    }),
  );
  await page.goto(`/s/${STATE.sessions[0].id}`);
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
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo, null, BUSY_BAR);
  await expectThumbTargets(page);
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

  // ⚠ **Not yet 'answered', and that is the fix.** The verdict used to be drawn
  // straight from the write reaching the pipe, so a session that had stopped
  // reading showed a green *answered* while still blocked on the same question
  // (memview #122). Nothing has come back from the session in this stream, so
  // the honest state is the intermediate one.
  await expect(page.locator('.verdict')).toHaveText('sent — not taken up yet');
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
  // Sent, not yet taken up — see the note in the test above.
  await expect(page.locator('.verdict')).toHaveText('sent — not taken up yet');
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

test('an answer does not pay for the newlines between its blocks @ phone width', async ({
  page,
}) => {
  // ⚠ **The container preserves whitespace and the content is markup.** `.entry`
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
  await page.route('**/api/sessions/*/events', (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: [
        { kind: 'started', model: 'x', cwd: '/home/example/Code/memview', tools: 1 },
        { kind: 'text', text: '## A heading\n' },
        { kind: 'text', text: '\nA paragraph of prose.\n' },
        { kind: 'text', text: '\n## Another heading\n' },
        { kind: 'text', text: '\nAnd a second paragraph.\n' },
        { kind: 'turn', cost_usd: 0, turns: 1, duration_ms: 1, at: NEXT },
      ]
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page.locator('.body').first().waitFor();

  const gaps = await page.evaluate(() => {
    const body = document.querySelector('.body');
    if (!body) return [];
    const kids = [...body.children];
    const found: { between: string; gap: number }[] = [];
    for (let i = 0; i < kids.length - 1; i++) {
      const above = kids[i].getBoundingClientRect();
      const below = kids[i + 1].getBoundingClientRect();
      found.push({
        between: `${kids[i].tagName}→${kids[i + 1].tagName}`,
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

  // ⚠ **With the wheel, not by writing `scrollTop`.** Setting the position from
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
  // ⚠ **The gap memview#116 lived in for three wrong theories.** Every following
  // check above moves the view with a wheel or not at all, and the defect was on
  // the path where a FINGER is on the glass: the engine decided per scroll event
  // while held, so eighteen pixels of thumb drift — measured on the phone while
  // deliberately not scrolling — ended following for good, and a live session
  // read as a dead page. It was settled by asking Pippijn to hold his phone,
  // which was the wrong instrument for a question a browser can answer.
  await handControlOfTheStream(page);
  await mockRunner(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page.locator('.transcript').waitFor();
  await say(page, { kind: 'text', text: LONG_ANSWER }, 1);
  expect(await distanceFromTheEnd(page), 'not at the end to begin with').toBeLessThan(4);

  // ⚠ **Watch how far the view actually travels, rather than trusting the
  // gesture.** Chromium eats an unknown amount of the finger's movement as
  // gesture slop, so a request to drag 30px can move the view by 30 or by
  // nothing — and "by nothing" would leave this test passing without ever having
  // exercised a hold. The synthesized gesture is atomic, so the furthest point
  // has to be collected as it happens.
  const furthest = async () =>
    page.evaluate(() => (window as unknown as { __min: number }).__min);
  await page.evaluate(() => {
    const el = document.querySelector('.transcript')!;
    const w = window as unknown as { __min: number };
    w.__min = el.scrollTop;
    el.addEventListener('scroll', () => (w.__min = Math.min(w.__min, el.scrollTop)));
  });
  const before = await page.evaluate(() => document.querySelector('.transcript')!.scrollTop);

  await thumb(page, 45);

  const moved = before - (await furthest());
  // ⚠ **The band is the test.** Only a movement ABOVE `SLACK` (16) and BELOW
  // `SLOP` (40) can tell the fix from its absence: under 16 the old engine would
  // not have unpinned either, so the check passes without exercising anything.
  // Measured while writing this — the first version asked for 30px, Chromium ate
  // 15 of it as gesture slop, the view moved 15, and the whole test survived the
  // fix being removed. These bounds fail loudly if that drifts again.
  expect(moved, 'under SLACK — the old engine would have forgiven this too').toBeGreaterThan(16);
  expect(moved, 'over SLOP — that is a scroll back, which is the test below').toBeLessThan(40);

  // ⚠ **`SLACK`, not the 4px the wheel checks use.** A real touch gesture leaves
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
  // ⚠ **Reported from the phone, 2026-08-11**: reaching the very end by hand and
  // sitting there, with the session still writing, left the page not following —
  // `gap=1 top=136287` and still declining to write, then 58 and 82 as the
  // conversation grew under a reader who had not moved. Once away, every move is
  // theirs, INCLUDING the one that comes back; the unit tests say so and nothing
  // said it through a real gesture.
  await handControlOfTheStream(page);
  await mockRunner(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page.locator('.transcript').waitFor();
  await say(page, { kind: 'text', text: LONG_ANSWER }, 1);

  await thumb(page, 300);
  const away = await distanceFromTheEnd(page);
  expect(away, 'the scroll back did not take').toBeGreaterThan(200);

  // Back down, overshooting so the box clamps at its own end rather than needing
  // the gesture to land on a pixel.
  await thumb(page, -600);
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
  // The other side of the same gesture, and what memview#82 exists to protect.
  // A hold is forgiven precisely because a drag is not, so forgiving one without
  // the other is not a fix, it is following that cannot be stopped by hand.
  await handControlOfTheStream(page);
  await mockRunner(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page.locator('.transcript').waitFor();
  await say(page, { kind: 'text', text: LONG_ANSWER }, 1);

  await thumb(page, 400);
  const away = await distanceFromTheEnd(page);
  expect(away, 'the scroll back did not take').toBeGreaterThan(200);

  await say(page, { kind: 'text', text: LONG_ANSWER }, 2);
  expect(await distanceFromTheEnd(page), 'yanked the reader to the end').toBeGreaterThan(away);
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
        sessions: [{ ...STATE.sessions[0], name: 'running', context: 496_231, window: 1_000_000 }],
      },
    }),
  );
  await page.route('**/api/past', (r) =>
    r.fulfill({ json: [{ ...ON_DISK[0], name: 'on-disk', context: 340_000 }] }),
  );
  await page.goto('/');
  await expect(page.locator('.session')).toHaveCount(2);

  const facts = await page.locator('.session .facts').allInnerTexts();
  expect(facts[0], 'a running session knows what it is full of').toContain('496k / 1M');
  expect(facts[1], 'a bare count with no denominator has to name its unit').toContain(
    '340k tokens',
  );
  // ⚠ **And the history is not here.** It was, and a fourth fact wrapped the row
  // onto a second line — so the size moved to the details sheet, which is where
  // a fact you look up belongs rather than one you scan a list by.
  expect(facts.join(' '), 'the size is back on the card').not.toContain('MB');

  // The row wraps rather than clipping, and what it must not do is push the card
  // sideways or land on top of itself.
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo);
});

test('session list — work still running says so, silence otherwise @ phone width', async ({
  page,
}, testInfo) => {
  // ⚠ **The one thing a card can be doing while it reads as idle.** A
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
            ...STATE.sessions[0],
            id: 'aaaa0000-0000-4000-8000-000000000001',
            name: 'a-session-with-a-name-long-enough-to-crowd',
            busy: undefined,
            waiting: 0,
            background: 1,
          },
          {
            ...STATE.sessions[0],
            id: 'aaaa0000-0000-4000-8000-000000000002',
            name: 'quiet',
            busy: undefined,
            waiting: 0,
          },
        ],
      },
    }),
  );
  await page.route('**/api/past', (r) => r.fulfill({ json: [] }));
  await page.goto('/');
  await expect(page.locator('.session')).toHaveCount(2);

  // ⚠ **In the head, beside the status word it qualifies** — not down in the
  // facts row with the dates and counts, where it read as one more piece of
  // history rather than as the thing still happening.
  await expect(
    page.locator('.session', { hasText: 'long-enough-to-crowd' }).locator('.head'),
  ).toContainText('1 background task');
  await expect(
    page.locator('.session', { hasText: 'long-enough-to-crowd' }).locator('.head'),
  ).toContainText('idle');
  // ⚠ Nothing at all for a session with none — `0 background tasks` would be a
  // claim, and the harness cannot see a command backgrounded inside a shell.
  await expect(page.locator('.session', { hasText: 'quiet' })).not.toContainText('background');
  await expect(page.locator('.tasks')).toHaveCount(1);

  // The other crowded head: a name long enough to push everything else along,
  // and two things qualifying the status word. See [[expectOneLine]].
  await expectOneLine(page, '.session .head');
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo);
});

test('session list — what each conversation still owes @ phone width', async ({
  page,
}, testInfo) => {
  // ⚠ **The list a session keeps for itself, on the page that lists sessions.**
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
          { ...STATE.sessions[0], id: 'aaaa0000-0000-4000-8000-000000000001', name: 'owing' },
          { ...STATE.sessions[0], id: 'aaaa0000-0000-4000-8000-000000000002', name: 'no-list' },
          { ...STATE.sessions[0], id: 'aaaa0000-0000-4000-8000-000000000003', name: 'all-done' },
          { ...STATE.sessions[0], id: 'aaaa0000-0000-4000-8000-000000000004', name: 'astray' },
        ],
        tasks: {
          sessions: {
            'aaaa0000-0000-4000-8000-000000000001': { open: 3, total: 17 },
            'aaaa0000-0000-4000-8000-000000000003': { open: 0, total: 4 },
            // A conversation nothing is running, which still has the list it kept
            // — and which migrated without deleting what it left behind.
            'bbbb0000-0000-4000-8000-000000000001': { open: 2, total: 9, stray: 5 },
            // ⚠ **Nothing in the service and eleven in the store it replaced.**
            // The case the sweep of holders cannot produce on its own: work
            // being filed where nothing reads it. No fraction, just the fault.
            'aaaa0000-0000-4000-8000-000000000004': { open: 0, total: 0, stray: 11 },
          },
          // The holders that are on no card, because they are not conversations.
          elsewhere: [
            { name: 'Pippijn', open: 1, total: 12 },
            { name: 'nobody', open: 23, total: 26 },
          ],
        },
      },
    }),
  );
  await page.route('**/api/past', (r) => r.fulfill({ json: ON_DISK }));
  await page.goto('/');
  await expect(page.locator('.session')).toHaveCount(6);

  await expect(page.locator('.session', { hasText: 'owing' }).locator('.list')).toContainText(
    '3/17',
  );
  // ⚠ **A conversation that is not running says it too.** The list is on disk
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
  // ⚠ And nothing at all where there is no list — `0/0` would claim one that had
  // been emptied, and most conversations never open one.
  await expect(page.locator('.session', { hasText: 'no-list' }).locator('.list')).toHaveCount(0);

  // ⚠ **What a conversation left in the store the service replaced.** Every file
  // there is re-sent to it 1.75 times per message with its whole body, so this
  // is a fault to clear rather than a quantity to know — in parentheses, beside
  // the fraction, and not part of it.
  // ⚠ Read as two elements, because the space between them is a margin and not
  // a character: Angular drops whitespace-only nodes, so the text is `2/9(5)`.
  const older = page.locator('.session', { hasText: 'older' });
  await expect(older.locator('.fraction')).toHaveText('2/9');
  await expect(older.locator('.stray')).toHaveText('(5)');
  // ⚠ **And with no fraction at all when the service has never heard of it** —
  // work being filed where nothing reads it, which is the case a sweep of the
  // service's own holders cannot produce.
  const astray = page.locator('.session', { hasText: 'astray' });
  await expect(astray.locator('.stray')).toHaveText('(11)');
  await expect(astray.locator('.fraction')).toHaveCount(0);
  // Nothing at all on a session that has cleaned up: the number only ever
  // appears when there is something to do about it.
  await expect(page.locator('.session', { hasText: 'owing' }).locator('.stray')).toHaveCount(0);

  // ⚠ **What is on no card, because it belongs to no conversation.** The pile is
  // the one queue nobody is working, and every other thing on this page is drawn
  // per session — so without this line it is invisible here by construction.
  await expect(page.locator('.elsewhere')).toContainText('Pippijn 1/12');
  await expect(page.locator('.elsewhere')).toContainText('nobody 23/26');

  // ⚠ **The assertion this feature earned, twice.** The chip shipped as an
  // inline-flex box, which reports its icon's baseline rather than its digits',
  // so the count rode 3.8px above everything beside it; and the row it joined
  // was aligned on baselines, which put the status pill 2px below the name. Two
  // different faults, both invisible to every other check here, both found by
  // eye on the phone — see [[expectOneLine]].
  await expectOneLine(page, '.session .head');
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo);
  await expectNoClippedText(page, testInfo);
});

test('session list — what each conversation is about, marked as a guess @ phone width', async ({
  page,
}, testInfo) => {
  // ⚠ **The only inference on the page.** Everything else on a card is read off
  // a file or a process; this sentence is Haiku's reading of the transcript, so
  // it carries a mark saying who wrote it and when. A guess about a conversation
  // nobody has opened is precisely the one somebody would act on unchecked.
  const written = Date.now() - 9 * 60_000;
  await mockRunner(page);
  await page.route('**/api/state', (r) =>
    r.fulfill({
      json: {
        ...STATE,
        sessions: [
          { ...STATE.sessions[0], id: 'aaaa0000-0000-4000-8000-000000000001', name: 'health' },
        ],
        gists: {
          'aaaa0000-0000-4000-8000-000000000001': {
            text: 'porting the last matcher gate to Lean and checking it against the golden set',
            at: written,
          },
          // One for a conversation that is not running, which is where it helps
          // most: a name nobody has opened in a week is a word.
          'bbbb0000-0000-4000-8000-000000000001': {
            text: 'reworking the scanner band probe after the axis turned out to be wrong',
            at: written,
          },
        },
      },
    }),
  );
  await page.route('**/api/past', (r) => r.fulfill({ json: [ON_DISK[0]] }));
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

  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo);
});

test('session list — the opening instruction stands in for a missing name @ phone width', async ({
  page,
}) => {
  // ⚠ **It is a fallback, not a fact about the session.** What the console keeps
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
            ...STATE.sessions[0],
            id: 'aaaa0000-0000-4000-8000-000000000001',
            asked: 'fix the gate',
          },
          {
            ...STATE.sessions[0],
            id: 'aaaa0000-0000-4000-8000-000000000002',
            name: 'health',
            asked: 'Proceed',
          },
        ],
      },
    }),
  );
  await page.route('**/api/past', (r) => r.fulfill({ json: [] }));
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
  // ⚠ **Two wrong signals that compounded.** A session running tools showed
  // `idle`, because `busy` is announced only when it CHANGES (#112) — and a
  // message written to it was invisible from here (#111). A message sent to a
  // session the page calls idle should land at once, so its not landing read as
  // a failure, and the same sentence went twice.
  await mockRunner(page);
  const busy = { ...STATE.sessions[0], busy: undefined, waiting: 0, working: true, unread: 2 };
  await page.route('**/api/state', (r) =>
    r.fulfill({ json: { ...STATE, sessions: [busy, STATE.sessions[1]] } }),
  );
  await page.goto('/');
  await page.getByText('working').first().waitFor();
  await expect(page.locator('.unread')).toHaveText('2 unread');
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo);
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
  await openTools(page);
  const unfold = page.getByRole('button', { name: /characters/ });
  await unfold.waitFor();
  await unfold.click();
  await page.getByText('quantiseLegCost', { exact: false }).first().waitFor();
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo, null, BUSY_BAR);
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
        usage: {
          ...STATE.usage,
          five_hour: { pct: 92, resets_in_ms: 3_600_000 },
        },
      },
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

  // ⚠ **A label that wraps is neither clipped nor overflowing**, so every
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

  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo);
  await expectNoClippedText(page, testInfo, '.usage');
});

test('a window that has already reset shows no figure @ phone width', async ({ page }) => {
  // ⚠ **The normal case, not an edge one.** The number comes from Claude Code's
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

/**
 * A session's own task list, in the proportions a real one has.
 *
 * Subjects run as sentences rather than labels — that is how the tools are used
 * — and one row has nothing written up behind it, which is the row that must not
 * offer to open.
 */
/**
 * A session's list, in the service's own words.
 *
 * ⚠ **`done` / `open` / `doing`, not `completed` / `pending` / `in_progress`.**
 * The second set is the built-in task tool's vocabulary, which this read from
 * disk until the lists moved to the service. This fixture kept saying
 * `completed` afterwards, so the sheet — which hides `status === 'done'` — put
 * finished work on a screen that claims not to show any, and the spec that would
 * have caught it is not run by the gate.
 *
 * `active_form` and `blocked_by` are gone with them: the service has neither
 * field, so nothing here may invent one. See `console/src/tasks.rs`.
 */
const TASKS = [
  // ⚠ **In the service's order, ranks and all** — `repo::list` is the only sort
  // there is, and the rows arrive already in it. So the ranked ones are placed
  // here where the service would put them: P0 above everything unranked, P3
  // BELOW it, because an unranked task sorts exactly where P2 does and "when
  // there is room" is not more pressing than four hundred nobody has read.
  {
    id: '7',
    subject: 'refresh the recovery bundle before the next machine',
    status: 'open',
    detailed: false,
    priority: 'P0',
  },
  {
    id: '2',
    subject: 'bless the golden set after the oracle moved',
    status: 'done',
    detailed: true,
  },
  {
    id: '100',
    subject: 'write the rule up',
    status: 'open',
    detailed: false,
  },
  // Unranked and ranked-below it, adjacent on purpose: the row with no chip is
  // the more urgent of the two, and drawing an absent rank as anything at all
  // would say the opposite.
  {
    id: '99',
    subject: 'tidy the fixture names',
    status: 'open',
    detailed: false,
    priority: 'P3',
  },
  {
    id: '101',
    subject: 'port the matcher gate',
    status: 'doing',
    detailed: true,
    // ⚠ **A deadline that is NOT overdue, beside one that is.** The pair is the
    // point: a date that has not been missed is a fact about the task, not a
    // problem with it, and the render has to keep the two apart. `overdue` is
    // absent here rather than false, which is how the service sends it.
    due: '2026-09-01',
  },
  {
    // Both marks at once, on a subject long enough to wrap — which is where
    // spelling either of them out costs the lines this sheet cannot spare.
    id: '412',
    subject:
      'reconcile the overnight decoder against the joint model and say which of the two owns journey reconstruction',
    status: 'open',
    detailed: false,
    due: '2026-08-01',
    overdue: true,
    blocked: true,
    blocked_on: ['92', '93'],
  },
  {
    // Blocked with the link still on it AFTER the blocker closed: the service
    // says `blocked: false` and keeps `blocked_on` as a record of how the work
    // went. Nothing may be drawn for it — see [[waitingOn]].
    id: '413',
    subject: 'was waiting on the effect language',
    status: 'open',
    detailed: false,
    blocked_on: ['92'],
  },
  // ⚠ **The fourth state, and the one the sheet used to get wrong.** `dropped`
  // is closed without ever being done, and this sheet filtered on
  // `status !== 'done'` — so a dropped task stood among the open ones wearing
  // the icon for a status the console has never heard of. Five of them were live
  // on the tasks session when it was found.
  {
    id: '54',
    subject: 'move the per-session repo claim into the service',
    status: 'dropped',
    detailed: false,
  },
];

/** A session the runner has finished reading: it has a name, a model id and a
 *  permission mode, which is what the toolbar and the sheet divide between them. */
const NAMED = {
  ...STATE,
  sessions: [
    {
      ...STATE.sessions[0],
      name: 'health',
      mode: 'bypassPermissions',
      // The two sizes: what the model still holds, and what has been said. 62 MB
      // of history under a context two thirds full is a conversation that has
      // forgotten most of itself.
      context: 640_000,
      window: 1_000_000,
      bytes: 65_011_712,
    },
    STATE.sessions[1],
  ],
  // What it is about, keyed by conversation the way the runner sends it. Long
  // enough that the card clamps it, which is the case the sheet exists for.
  gists: {
    '6f7c2f11-0000-4000-8000-000000000001': {
      text:
        'porting the last of the matcher gate to Lean, proving it bit-exact against the ' +
        'TypeScript twin, and running the golden set to see which journeys moved',
      at: 1785600000000,
    },
  },
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

test('the bar is a session bar before the runner has answered @ phone width', async ({ page }) => {
  // ⚠ **A cold launch lands inside a conversation** — the wrapper reopens on the
  // page it remembers — and the toolbar used to decide which screen it was on by
  // whether `/api/state` had come back yet. For that one round trip it drew the
  // LIST's bar, so opening the app flashed `console` and a terminal glyph and
  // then swapped the whole row. Reported from the phone, where it is a fifth of
  // a second of the wrong screen every single time.
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
  await page.goto(`/s/${STATE.sessions[0].id}`);

  // Nothing has answered yet, and the way out is already on screen.
  await expect(page.locator('.bar .leave'), 'the bar is still the list’s').toBeVisible();
  await expect(page.locator('.bar .title'), 'the root headline flashed up').toHaveCount(0);

  answer?.();
  await expect(page.locator('.bar .name')).toHaveText('health');
});

test('a session with no name yet says where it runs, and which one it is @ phone width', async ({
  page,
}) => {
  // The state every session starts in: the runner has not read a name out of the
  // transcript, and the bar still has to say which conversation this is.
  //
  // ⚠ **The folder is not enough on its own any more.** Every session is started
  // in `~/Code`, the parent of every repository, so the folder answers the same
  // for all of them — the short id is what tells two new sessions apart, and it
  // is also what claims a task list.
  await mockRunner(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await expect(page.locator('.bar .name')).toHaveText('decode · 6f7c2f11');
  await expect(page.locator('.bar .name')).toHaveClass(/anonymous/);
});

test('the list says nothing about which machine it is @ phone width', async ({ page }) => {
  // "this Mac" was true of every session on the list and news to nobody. What
  // replaced it is nothing about a session: everything the ⋮ menu offers acts on
  // one, and on the list there is no session on screen.
  //
  // ⚠ **This used to assert the bar held no button at all**, which was a
  // stronger claim than the reason above supports and it stopped being true.
  // Keeping the screen on is about the screen, not about a session, so it is
  // offered here as well — see the test below. What must stay absent is anything
  // aimed at a session that has not been chosen yet.
  await mockRunner(page);
  await page.goto('/');
  await page.getByText('decode').first().waitFor();
  await expect(page.locator('.bar')).not.toContainText('Mac');
  await expect(page.locator('.bar button[aria-haspopup="menu"]')).toHaveCount(0);
  await expect(page.locator('.bar .leave')).toHaveCount(0);
});

/**
 * Give the page a wake lock that always works, before anything runs.
 *
 * ⚠ **Stubbed rather than used.** Headless Chromium has no display to keep on
 * and refuses the real request, which the app correctly treats as a refusal and
 * puts the button back — so a test of the wiring would fail on the one thing it
 * is not testing. What Android actually does with a lock is a phone's answer,
 * not a harness's; this asserts the console asks, draws the answer, and offers
 * the control in both places.
 */
async function withWakeLock(page: Page): Promise<void> {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'wakeLock', {
      configurable: true,
      value: {
        request: () => Promise.resolve({ released: false, release: () => Promise.resolve() }),
      },
    });
  });
}

test('the screen can be kept on from either screen @ phone width', async ({ page }) => {
  // Watching a session work is done with no hands for minutes at a time, and the
  // phone's display timeout cannot tell looking from idling. The choice belongs
  // wherever somebody is when they make it — the session they are about to
  // watch, or the list they are about to pick one from.
  await withWakeLock(page);
  await mockRunner(page);

  for (const url of ['/', `/s/${STATE.sessions[0].id}`]) {
    await page.goto(url);
    const button = page.locator('.bar .awake');
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
    // ⚠ **Off the PROTOTYPE, not off the instance.** `wakeLock` is an accessor on
    // `Navigator.prototype`, so `delete navigator.wakeLock` removes an own
    // property that was never there and silently succeeds — the first version of
    // this test did exactly that and asserted against a browser that still had
    // the API. `delete` rather than assigning undefined, because the app asks
    // with `in`, which an own property set to undefined satisfies.
    delete (Navigator.prototype as { wakeLock?: unknown }).wakeLock;
  });
  await mockRunner(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page.locator('.bar .name').waitFor();
  await expect(page.locator('.bar .awake')).toHaveCount(0);
});

/** Where the toolbar's leading glyph starts — the terminal mark, or the arrow. */
async function leadingGlyph(page: Page): Promise<number> {
  return page.evaluate(() => document.querySelector('.bar mat-icon')!.getBoundingClientRect().left);
}

test('the toolbar starts in the same place on both screens @ phone width', async ({ page }) => {
  // Entering a session and leaving one swaps the leading glyph — the terminal
  // mark for a back arrow — and the eye tracks a mark that stays put.
  //
  // ⚠ **They do not line up by default, and the reason is invisible in the
  // markup.** An icon button carries 8px of padding inside its own box and a
  // bare `mat-icon` carries none, so the arrow begins 8px further in than the
  // mark it replaces. Nothing overflows, nothing is clipped, and no check that
  // measures one screen at a time can see it: the fault is a difference BETWEEN
  // two renders, which is why this test loads both.
  await mockRunner(page);
  await page.goto('/');
  // `.first()`, because the bar now carries a second glyph: the screen-awake
  // control sits at the end of both branches, and a bare locator matching two
  // elements is a strict-mode violation rather than a measurement.
  await page.locator('.bar mat-icon').first().waitFor();
  const list = await leadingGlyph(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page.locator('.bar .leave').waitFor();
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
  // and where it lands, not about the page.
  //
  // ⚠ **And the second version passed against it too, for the opposite reason.**
  // The name is a block heading now rather than an inline label, and a block's
  // rect is its BOX — it does not grow with text that spills out of it, the way
  // the old span's did. So every box measurement below stays true while the
  // glyphs paint straight over the ⋮: ablating `overflow: hidden` off `.name`
  // was measured to change nothing here. A rect cannot see this class of fault
  // at all, which is why the clipping is asserted directly.
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
    const menu = document
      .querySelector('.bar button[aria-haspopup="menu"]')!
      .getBoundingClientRect();
    const leave = document.querySelector('.bar .leave')!.getBoundingClientRect();
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
  // ⚠ **The check this suite could not make, because the fixture had no mode.**
  // Both the card and the session header draw the glyph only for a session whose
  // mode the runner has read, so with `STATE.sessions[0]` carrying none it was
  // absent from every render the harness measured — and `expectThumbTargets`,
  // `expectIconsCentred`, the overlap and overflow passes were all measuring a
  // row missing an element.
  //
  // What that cost: making the glyph a button inherited the app-wide 3rem floor
  // from `styles.scss`, the header went 19px → 40px, and the FULL GATE PASSED.
  // It was caught by looking at the render (memview #633).
  //
  // Asserted by presence and by box rather than by text: the neighbouring
  // `.facts` checks all use `toContain`, so an extra ligature slips past them
  // and so does its disappearance.
  await mockRunner(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
  const mode = page.locator('.head .mode');
  await expect(mode).toHaveCount(1);
  // The name is what the icon means, and it is the only thing that says so on
  // this row — the label is one tap away in the menu that also teaches it.
  await expect(mode).toHaveAttribute('aria-label', /edits/i);

  // ⚠ **In the row, not overhanging it.** The regression was exactly this: a
  // glyph whose box grew past the line it sits on, pushing the header's height
  // and shoving the model name right.
  // ⚠ **Against the text beside it, NOT against its own row.** The first
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
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo);
  await expectThumbTargets(page);
});

test('the session is still named after scrolling to the end @ phone width', async ({ page }) => {
  // Which conversation you are in is the one fact worth having on screen at all
  // times — the console drives a dozen at once and they differ by name. Both the
  // toolbar and the facts row sit outside the scrolling region (`.transcript` is
  // the only thing on this page that scrolls), so this measures that they do.
  await mockRunner(page);
  await page.route('**/api/state', (r) => r.fulfill({ json: NAMED }));
  await page.goto(`/s/${STATE.sessions[0].id}`);
  // Opened, because this needs a transcript tall enough to scroll and a folded
  // run is deliberately short.
  await openTools(page);
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
  // Behind the ⋮, where everything else you can do to a session already was.
  await page
    .locator('.bar')
    .getByRole('button', { name: /what to do with/ })
    .click();
  await page.getByRole('menuitem', { name: 'Details' }).click();
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
  // How much has been said, which the list card used to carry and cannot: four
  // facts in that row wrapped it. Beside how full the context is, because the
  // gap between the two is the fact neither one states.
  expect(said).toContain('62 MB');
  expect(said).toContain('640k / 1M');
  // ⚠ **Whole, and attributed.** The card clamps this sentence to two lines, so
  // being cut there is a reason to open the sheet — finding it cut here as well
  // would be the panel failing at its one job. And it is the only line in it a
  // model wrote, which the sheet has room to say in words.
  expect(said).toContain('running the golden set to see which journeys moved');
  expect(said).toContain('written by Haiku');

  await expectNoTextOverlaps(page, testInfo, '.session-sheet');
  await expectNoHorizontalOverflow(page, testInfo, '.session-sheet');
  await expectNoClippedText(page, testInfo, '.session-sheet');
});

test('the task sheet opens on what is left rather than what is done @ phone width', async ({
  page,
}, testInfo) => {
  // ⚠ **The list is mostly finished work, by an order of magnitude.** One live
  // session here keeps 355 tasks: 307 done, 38 open, 10 underway. Opening on all
  // of them puts three hundred completed rows above the eight that matter, so
  // the default is what is left — and the toggle is there because the finished
  // ones are a written record worth reading, not because the list is a total.
  await mockRunner(page);
  await page.route('**/api/sessions/*/tasks', (r) => r.fulfill({ json: TASKS }));
  // ⚠ **Markdown, with a code block in it.** These are written up as reports —
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
      },
    }),
  );
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page
    .locator('.bar')
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
  // writes `#101 done` in its own prose, so the row has to be findable by it.
  //
  // ⚠ **And in the service's order within a status, not one of ours.** #7 is
  // ranked above the unranked #100 and #99 is ranked below it, and that is how
  // they arrive — nothing here re-sorts on the rank, or there would be two
  // orderings to keep true and they would disagree the first time either moved.
  await expect(sheet.locator('.num')).toHaveText(['101', '7', '100', '99', '412', '413']);

  // ⚠ **A deadline is drawn, and it does NOT reorder anything.** #412 is overdue
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

  // ⚠ **One block mark, not two.** #413 still carries `blocked_on` and the
  // service says it is no longer blocked — the link is kept after a blocker
  // closes, as a record of how the work went. A row deciding for itself from the
  // ids would say the opposite, and this client has no way to know better.
  await expect(sheet.locator('.held')).toHaveCount(1);
  await expect(sheet.locator('.held')).toHaveAttribute('aria-label', 'waiting on #92, #93');

  // ⚠ **The rank is drawn, and only where there is one.** Two chips on four
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
  // ⚠ **Eight — the six open ones and the two that were closed.** This asked for
  // six, which is the count BEFORE the click, and passed for two months because
  // `toHaveCount` polls: the first poll landed before change detection had
  // appended the closed rows, matched the stale DOM and returned. Under load the
  // first poll arrives after the render instead, sees the truth and fails — read
  // as "the sheet opened on All" in three gate runs (#735) when the sheet had
  // done nothing wrong and the assertion was measuring its own race.
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

  await expectNoTextOverlaps(page, testInfo, '.session-sheet');
  await expectNoHorizontalOverflow(page, testInfo, '.session-sheet');
  await expectNoClippedText(page, testInfo, '.session-sheet');
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
  const at = `/s/${STATE.sessions[0].id}`;
  await page.goto('/');
  await page.locator('.session').first().click();
  await page.locator('.transcript').waitFor();
  await page
    .locator('.bar')
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
  // ⚠ **The other half of giving an overlay a history entry**, and the half that
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
    .locator('.bar')
    .getByRole('button', { name: /what to do with/ })
    .click();
  await page.getByRole('menuitem', { name: 'Details' }).click();
  await page.locator('.session-sheet').waitFor();

  // Away by hand, not by back: the backdrop is what a thumb reaches first.
  // `.last()` is the sheet's own — the menu that opened it leaves its backdrop in
  // the DOM behind this one while it fades.
  //
  // ⚠ Aimed at the top of the screen rather than clicked at its centre. The
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
  await page.locator('.bar .leave').click();
  await page.locator('.session').first().waitFor();
  answer?.();
  // Long enough for the held response to land and be acted on.
  await page.waitForTimeout(300);
  await expect(
    page.locator('.bar .name'),
    'the list is titled with the session just left',
  ).toHaveCount(0);
  // ⚠ **Narrowed from "no buttons at all" once the screen-awake control arrived.**
  // What this test is about is a session that is no longer on screen still being
  // actionable — so the assertion is about the controls that act on ONE, not
  // about the toolbar being empty. Keeping the screen on acts on the screen and
  // is offered on the list by design; see the test that covers it.
  await expect(
    page.locator('.bar button[aria-haspopup="menu"]'),
    'the ⋮ still acts on the session just left',
  ).toHaveCount(0);
  await expect(page.locator('.bar .leave')).toHaveCount(0);
});

test('a run of tool calls is folded into one row @ phone width', async ({ page }) => {
  // ⚠ **Machinery is what a reader scrolls past.** A tool call is 115px at phone
  // width and a turn can hold a dozen, so a conversation with any work in it is
  // mostly rows nobody came for, between the two sentences they did.
  await mockRunner(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
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
  // ⚠ **A path with no verb attached says nothing that matters.** `Read` and
  // `Write` on the same file are opposite events, and the argument alone cannot
  // tell them apart — nor can it say a file was deleted rather than looked at.
  //
  // ⚠ **This is a REGRESSION test, and the shape of it is the point.** The name
  // was dropped from the standalone row in 65607c7, when the argument grew a
  // parse button and the row became two branches; the FOLDED run kept its copy,
  // so every existing assertion about tool rows still passed while a lone call
  // showed a bare path. Hence the seed below is one call between prose: a run of
  // two folds (`A_RUN`), and a folded row would exercise the wrong markup and
  // pass against the defect.
  await handControlOfTheStream(page);
  await mockRunner(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page.locator('.transcript').waitFor();
  let seq = 0;
  await say(page, { kind: 'said', text: 'Let me look at the gate.' }, ++seq);
  await say(
    page,
    { kind: 'tool', id: 'lone', name: 'Read', input: { file_path: '/home/example/gate.ts' } },
    ++seq,
  );
  await say(page, { kind: 'said', text: 'That is the one.' }, ++seq);

  const row = page.locator('.entry').filter({ hasText: '/home/example/gate.ts' }).last();
  await expect(row.locator('.tool-name'), 'the row names its tool').toHaveText('Read');
  await expect(row.locator('.tool-arg')).toContainText('gate.ts');
});

test('a running thing says how long it has been running @ phone width', async ({ page }) => {
  // ⚠ **"running" is the same word at four seconds and at forty minutes**, and
  // only one of those is worth interrupting. The bar above the composer had the
  // same problem: it says something is happening and could not say for how long,
  // which is the difference between waiting and wondering whether it is stuck.
  await handControlOfTheStream(page);
  await mockRunner(page);
  // ⚠ **Working and blocked are not the same session.** The shared fixture holds
  // a standing question, and the strip is deliberately absent while one is on
  // screen — a session waiting on a permission is not one whose output is
  // arriving. See `SessionView.arriving`. This test is about the other case, so
  // it takes the question away rather than asserting a combination the runner
  // cannot produce.
  await page.route('**/api/state', (r) =>
    r.fulfill({
      json: {
        ...STATE,
        sessions: [{ ...STATE.sessions[0], waiting: 0 }, ...STATE.sessions.slice(1)],
      },
    }),
  );
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page.locator('.transcript').waitFor();
  let seq = 0;
  // Stamped two minutes ago, so what is counted from does not depend on how
  // fast this test runs.
  const began = Date.now() - 125_000;
  await say(page, { kind: 'busy', status: 'requesting', at: began }, ++seq);
  await say(
    page,
    { kind: 'tool', id: 'slow', name: 'Bash', input: { command: 'cargo build' }, at: began },
    ++seq,
  );

  // ⚠ **The strip is the bar and the clock, and no word.** The status the CLI
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
  // ⚠ **Two versions of opening it automatically were tried and cut.** Reading
  // `running > 0` live flickers — a session making one call at a time turns a
  // pair into a run and opens it, the result empties the run and folds it, the
  // next call opens it again, reported from the phone as "it keeps flipping open
  // and closed". Latching that condition stopped the flicker and cost more: the
  // page was no longer a function of the conversation, since what was open
  // depended on whether you had been watching.
  //
  // What the automatic open was for, the summary row does on its face.
  // ⚠ Before `goto`: it installs its stub through `addInitScript`, which only
  // affects pages loaded after it.
  await handControlOfTheStream(page);
  await mockRunner(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page.locator('.transcript').waitFor();
  let seq = 0;
  for (const event of [
    { kind: 'tool', id: 'run_a', name: 'Bash', input: { command: 'cargo build --workspace' } },
    { kind: 'tool', id: 'run_b', name: 'Bash', input: { command: 'cargo test --all-features' } },
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

  // ⚠ **And the result that lands next does not close it again.** This is the
  // flicker, from the other side: a run opened by hand has to stay open when
  // the condition that used to drive it changes under the reader.
  await say(page, { kind: 'tool_result', id: 'run_b', ok: true, detail: 'ok' }, ++seq);
  await expect(run, 'the run has finished').not.toContainText('2 running');
  await expect(page.getByText('cargo test --all-features')).toBeVisible();
});

test('one event does not rebuild the rows already on screen @ phone width', async ({ page }) => {
  // ⚠ **The defect this exists for, and it was invisible.** `blocks()` wraps each
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
  await page.goto(`/s/${STATE.sessions[0].id}`);
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
  // ⚠ **Reported twice from the phone, and the second report named the number:**
  // "I need to scroll up quite a lot, then it won't do that". Following used to
  // re-decide after every change whether the reader still counted as being at
  // the end, and needed 300px of slack to survive the browser's own adjustments
  // — so a small scroll up ended with the page pulling itself back down. A line
  // is what stops a terminal, a messages app and a chat client following.
  await handControlOfTheStream(page);
  await mockRunner(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
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

  // Up by less than a line and a half — the movement that used to be treated as
  // no movement at all.
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
  await page.goto(`/s/${STATE.sessions[0].id}`);
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

/**
 * The runner's real answer for the transcript fixture's own failed command.
 *
 * ⚠ **Not invented.** Produced by `console::parse::parsed` for exactly that
 * command line, in exactly that session's directory, with `ok: false` — and
 * pinned from the Rust side by `the_shape_the_phone_is_drawn_from` in
 * `console/tests/parse.rs`, so a change in the reader breaks a test instead of
 * leaving this file quietly drawing something the runner would never send.
 *
 * It is the interesting case rather than a tidy one: the call failed, so
 * everything after the `&&` parses, classifies, names a real path — and none of
 * it is certain. That is the whole reason the sheet exists.
 */
const PARSED = {
  steps: [
    {
      depth: 0,
      argv: ['nix', 'develop', '-c', 'lake', 'build'],
      reached: 'always',
      cwd: '/home/example/Code/health/packages/health-sync-backend/src/decode',
      kind: 'nothing',
    },
    {
      depth: 0,
      argv: ['./verified_cli', 'match', '--serve', '--timeout', '30000ms'],
      reached: 'on-success',
      cwd: '/home/example/Code/health/packages/health-sync-backend/src/decode',
      kind: 'run',
      uses: [
        {
          path: '/home/example/Code/health/packages/health-sync-backend/src/decode/verified_cli',
          write: false,
          reached: 'on-success',
          certain: false,
        },
      ],
    },
    {
      depth: 0,
      argv: ['tee', '/tmp/lean-gate.log'],
      reached: 'on-success',
      cwd: '/home/example/Code/health/packages/health-sync-backend/src/decode',
      kind: 'write',
      uses: [{ path: '/tmp/lean-gate.log', write: true, reached: 'on-success', certain: false }],
    },
  ],
};

/** As [[mockRunner]], and answering the parse the sheet asks for. */
async function mockParse(page: Page, answer: unknown = PARSED): Promise<void> {
  await page.route('**/api/sessions/*/parse', (r) => r.fulfill({ json: answer }));
}

/**
 * The sheet, and the scope every layout check on it is given.
 *
 * ⚠ **An overlay legitimately covers the page it is over.** Unscoped, the
 * overlap check compares this sheet's text against the transcript still rendered
 * behind it and reports seventeen collisions that are the whole point of a
 * bottom sheet. Same reasoning, and the same argument, as the details sheet.
 */
const SHEET = '.session-sheet';

/**
 * Open the parse of the fixture's failed shell command, with the sheet settled.
 *
 * ⚠ **Waits for the animation, not for the element.** A Material bottom sheet
 * slides up, so a locator is visible — and measurable, and wrong — while it is
 * still 300px below where it will come to rest. That is not flakiness to retry
 * around: it is a box that has not finished moving, and the fix is to wait for
 * the thing that is moving it.
 */
async function openParse(page: Page): Promise<void> {
  await openTools(page);
  await page.getByRole('button', { name: /verified_cli/ }).click();
  const sheet = page.locator(SHEET);
  await sheet.waitFor();
  // ⚠ **Wait for the ANSWER, not for the sheet and not for `.raw`.** The sheet
  // is attached first and holds a progress bar until `/api/sessions/*/parse`
  // replies; `.raw` is drawn from the command it was opened with and is there
  // the whole time. So waiting on either leaves `settleTransforms` nothing to
  // wait for — it finds nothing running and returns at once — and the check
  // downstream read `.step` as `null` on a page that was about to be correct.
  //
  // Three shapes, because all three are answers: the summary, *this one does not
  // parse* (0.4% of the corpus, and its own test here), and the fetch having
  // failed. Waiting on `.step` instead hung that middle case for 30s — a parse
  // with no steps is exactly what it is about.
  await page
    .locator(`${SHEET} .summary, ${SHEET} .unread, ${SHEET} .trouble`)
    .first()
    .waitFor();
  // ⚠ **`settleTransforms`, and the two earlier waits here were both wrong.**
  //
  // The first asked `sheet.getAnimations()` and got nothing, and the note left
  // behind said the slide was invisible to `getAnimations` — it is not. The
  // animation is on the ANCESTOR `mat-bottom-sheet-container`, and an element
  // does not report its ancestors' animations; `document.getAnimations()` lists
  // it, `iterations: 1`, with `transform` in its keyframes, which is exactly
  // what `settleTransforms` already selects.
  //
  // The second watched this element's `top` for two equal frames. That settled
  // on the first pair, every time, because **this element's top never moves**:
  // traced at 8× throttle it sat at 387 from frame 0 while `.step`'s bottom
  // travelled 907 → 577 over eleven frames. So the box was measured 330px from
  // where it comes to rest, which is the 934.3 > 839 and 898.2 > 839 this
  // sheet's layout check failed on in two gate runs (#735).
  await settleTransforms(page);
}

test('a shell command opens as written and as read @ phone width', async ({ page }, testInfo) => {
  // ⚠ **The two halves have to be comparable without scrolling between them.**
  // That is the whole reason they are stacked rather than switched between, and
  // it is a claim about a rendered page that no amount of reading the source can
  // settle: the raw text is unbreakable path-shaped strings and the parse below
  // it is a list of the same.
  await mockRunner(page);
  await mockParse(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await openParse(page);

  // The command entire, which the row it was opened from could only ellipsise.
  const raw = page.locator('.raw');
  await expect(raw).toContainText('nix develop -c lake build');
  await expect(raw).toContainText('tee /tmp/lean-gate.log');

  // And the parse under it, in the shell's own order.
  await expect(page.locator('.step')).toHaveCount(3);
  await expect(page.locator('.step .kind').nth(2)).toHaveText('write');

  // ⚠ **The claim the sheet is for.** The call failed, so the write after the
  // `&&` may never have happened — and the summary says how many such uses there
  // are without hiding any of them.
  await expect(page.locator('.summary')).toContainText('2 unproven');
  await expect(page.locator('.used.unsure')).toHaveCount(2);

  await expectNoTextOverlaps(page, testInfo, SHEET);
  await expectNoHorizontalOverflow(page, testInfo, SHEET);
  await expectNoClippedText(page, testInfo, SHEET);
});

test('both halves of a parsed command fit one screen @ phone width', async ({ page }) => {
  // The layout claim, measured rather than eyeballed: the raw text and the first
  // classified step have to be on screen together, or the sheet has failed at
  // the one thing it was stacked this way to do.
  await mockRunner(page);
  await mockParse(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
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
  await mockParse(page, { error: 'a case arm', steps: [] });
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await openParse(page);

  await expect(page.locator('.unread')).toContainText('does not parse');
  await expect(page.locator('.step')).toHaveCount(0);
  await expectNoHorizontalOverflow(page, testInfo, SHEET);
});

test('another machine is named on the step and on every path @ phone width', async ({
  page,
}, testInfo) => {
  // ⚠ **A path that looks local and is not is the mistake the whole separation
  // exists to prevent** — so it is said twice, and a reader scrolling fast sees
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
      },
    ],
  });
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await openParse(page);

  await expect(page.locator('.step').nth(1).locator('.host')).toContainText('isis');
  await expect(page.locator('.used .elsewhere')).toContainText('on isis');
  // Indented, so the wrapper is visibly not the thing that read the file.
  const indents = await page.evaluate(() =>
    [...document.querySelectorAll('.step')].map((s) => s.getBoundingClientRect().left),
  );
  expect(indents[1], 'the command inside the wrapper is not indented').toBeGreaterThan(indents[0]);
  await expectNoHorizontalOverflow(page, testInfo, SHEET);
  await expectNoClippedText(page, testInfo, SHEET);
});

test('a session that has stopped reading names it, with the cure @ phone width', async ({
  page,
}, testInfo) => {
  // ⚠ **The row this replaces said the ordinary thing.** A deaf session's
  // messages carry the same *waiting to be read* marker as a busy session's, so
  // both of 2026-08-08's episodes cost a diagnosis by hand. This is the banner
  // that says which it is — see `session::Session::deaf`.
  //
  // At phone width because that is where it has to fit: a sentence, a duration
  // and a button on one row beside a transcript that is already tight.
  await mockRunner(page);
  const deaf = { ...STATE.sessions[0], busy: undefined, waiting: 0, unread: 2, deaf: 1284 };
  await page.route('**/api/state', (r) =>
    r.fulfill({ json: { ...STATE, sessions: [deaf, STATE.sessions[1]] } }),
  );
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page.locator('.deaf').waitFor();
  await expect(page.locator('.deaf')).toContainText('Not reading');
  await expect(page.locator('.deaf')).toContainText('21m');
  await expect(page.getByRole('button', { name: 'Restart it' })).toBeEnabled();
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo, null, BUSY_BAR);
  await expectNoPinnedOverlap(page);
});

test('an ended session offers the way back @ phone width', async ({ page }, testInfo) => {
  // The list has no way back from an ended session: the roster keeps it, so its
  // card stays a link to this page, and the resume affordance on the front page
  // exists only for a conversation the console is NOT holding. This is it.
  await mockRunner(page);
  await page.goto(`/s/${STATE.sessions[1].id}`);
  await page.locator('.deaf.ended').waitFor();
  await expect(page.getByRole('button', { name: 'Start it again' })).toBeEnabled();
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo, null, BUSY_BAR);
});

test('the verdict becomes the plain one once the session acts on it @ phone width', async ({
  page,
}, testInfo) => {
  // The other half of #122: *sent — not taken up yet* has to actually resolve.
  // The receipt is the session speaking, because a question blocks the turn — so
  // one tool call after the answer is proof it was read.
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
          reply: { answers: { 'How far should the question UI go?': 'options only' } },
        },
        { kind: 'tool', id: 'toolu_after', name: 'Bash', input: { command: 'echo taken up' } },
      ]
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page.locator('.chose').waitFor();

  await expect(page.locator('.verdict')).toHaveText('answered');
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo, null, BUSY_BAR);
});

test('a working session can be renamed from the menu @ phone width', async ({ page }, testInfo) => {
  // ⚠ **`/rename` cannot do this.** A slash command sent to a busy session is
  // parked and released as a prompt, so the model reads the words and the name
  // never changes — measured 2026-08-08. This route is a control request, which
  // the CLI answers mid-turn.
  let sent: Record<string, unknown> | undefined;
  await mockRunner(page);
  await page.route('**/api/sessions/*/rename', (r) => {
    sent = r.request().postDataJSON() as Record<string, unknown>;
    return r.fulfill({ json: STATE.sessions[0] });
  });
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page
    .locator('.bar')
    .getByRole('button', { name: /what to do with/ })
    .click();
  await page.getByRole('menuitem', { name: 'Rename' }).click();

  const name = page.getByLabel('name');
  await expect(name, 'the sheet did not open').toBeVisible();
  await name.fill('tasks');
  await expectNoHorizontalOverflow(page, testInfo, 'mat-bottom-sheet-container');
  await expectNoClippedText(page, testInfo, 'mat-bottom-sheet-container');
  await page.getByRole('button', { name: /^rename$/ }).click();

  await expect.poll(() => sent).toMatchObject({ title: 'tasks' });
  // Dismissed rather than redrawn: the new name arrives from the transcript on
  // the next poll, and claiming it from this response would be the console
  // reporting its own intent again.
  await expect(page.locator('mat-bottom-sheet-container')).toHaveCount(0);
});

test('the permission modes are one row that opens a sheet @ phone width', async ({
  page,
}, testInfo) => {
  // Six modes and a heading were most of the menu, so Details and Tasks — the
  // two things it is actually opened for — sat above a wall of settings.
  await mockRunner(page);
  // With a mode set, which every real session has — the runner records one at
  // spawn. The row shows it, so the state is readable without opening anything.
  const onAuto = { ...STATE.sessions[0], mode: 'acceptEdits' };
  await page.route('**/api/state', (r) =>
    r.fulfill({ json: { ...STATE, sessions: [onAuto, STATE.sessions[1]] } }),
  );
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page
    .locator('.bar')
    .getByRole('button', { name: /what to do with/ })
    .click();
  await expect(page.locator('.session-menu .current')).toHaveText('Accept edits');
  await expect(page.getByRole('menuitem', { name: /Asks permission/ })).toBeVisible();
  // ⚠ **Rename must not wear the `acceptEdits` pencil**, which is the row
  // directly below it — the same glyph a thumb apart, meaning two unrelated
  // things. Reported from the phone 2026-08-08.
  const icon = (name: RegExp) =>
    page.getByRole('menuitem', { name }).locator('mat-icon').first().textContent();
  expect(await icon(/Rename/)).not.toBe(await icon(/Asks permission/));
  await page.getByRole('menuitem', { name: /Asks permission/ }).click();
  const sheet = page.locator('mat-bottom-sheet-container');
  await sheet.waitFor();
  await expect(sheet.locator('.mode')).toHaveCount(6);
  // ⚠ Waiting on the last row being IN the viewport is what says the sheet has
  // finished arriving. Not because the slide is invisible to `getAnimations` —
  // it is not, see [[openParse]] — but because this row has to be reachable, not
  // merely still: a sheet that has come to rest with its last mode below the
  // fold is the defect this line is here for.
  await expect(sheet.locator('.mode').last()).toBeInViewport();
  await expectNoHorizontalOverflow(page, testInfo, 'mat-bottom-sheet-container');
  await expectNoClippedText(page, testInfo, 'mat-bottom-sheet-container');
  await expectThumbTargets(page);
});

test('session strip — a background call is named, not counted @ phone width', async ({
  page,
}, testInfo) => {
  // ⚠ **The count was the whole report, and it sent Pippijn to `ps`.** A phone
  // saying *1 background task running* cannot say WHICH, and one task with a
  // name is actionable where a number is only a reason to ask (memview #740).
  //
  // Both labels here are real shapes: `Monitor` carries a written description,
  // and a `Bash` one-liner arrives long enough to overflow a 412px line — which
  // is why the runner cuts to 60 and the strip ellipsises on top of that.
  await mockRunner(page);
  const id = STATE.sessions[0].id;
  await page.route('**/api/state', (r) =>
    r.fulfill({
      json: {
        ...STATE,
        sessions: [
          {
            ...STATE.sessions[0],
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
      },
    }),
  );
  await page.goto(`/s/${id}`);

  // The tool and its label, rather than a tally.
  const strip = page.locator('.update.running');
  await expect(strip).toHaveCount(2);
  await expect(strip.first()).toContainText('Monitor');
  await expect(strip.first()).toContainText('HDD→SSD migration');
  await expect(strip.nth(1)).toContainText('Bash');
  // ⚠ And the old wording is GONE when names are available — both would be the
  // same fact said twice, and the number is the half that could not be acted on.
  // Asserted as the ABSENCE OF THE FALLBACK ROW rather than as text `.update`
  // does not contain: `.update` resolves to two elements here, and a text
  // assertion against a multi-element locator is a strict-mode violation that
  // times out instead of failing on what it was actually asked.
  await expect(page.locator('.update:not(.running)')).toHaveCount(0);

  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo);
});
