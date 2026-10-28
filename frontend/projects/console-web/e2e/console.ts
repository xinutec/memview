import { expect, test, type Locator, type Page, type Route } from '@playwright/test';

// The golden the Rust test writes — see the note on PARSED below.
import PARSED_GOLDEN from './parsed.fixture.json';
import type {
  Call,
  Commands,
  Conversation,
  CorpusRead,
  Described,
  EditRecord,
  Landmark,
  LoginCode,
  LoginResult,
  LoginStarted,
  Message,
  Overview,
  Page as Earlier,
  Parsed,
  Refusal,
  Returned,
  Run,
  SudoAnswer,
  Summary,
  Stretched,
  Task,
  Timed,
} from '../src/app/models';
import { first, last, nth } from '../src/app/testing';
// The fleet-shared harness, published as @xinutec/ui-harness (source repo
// ~/Code/ui-harness). Ships compiled JS, so it loads straight from node_modules.
import {
  expectBackClosesOverlay,
  expectCleanLayout,
  expectIconFontLoaded,
  expectNoClippedText,
  expectNoHorizontalOverflow,
  expectUpInTheBar,
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

/** Material's indeterminate progress bar animates by translating its two bars
 *  right across the viewport and off the left edge — that is how the animation
 *  works, not a layout fault, and it is only on screen while a session is
 *  working. Allowed by selector rather than by turning the check off, so a real
 *  overflow on the same page still fails. */
/**
 * The runner's own answer for a recorded command, as `console/tests/suite/parse.rs`
 * pins it against the Rust side. A JSON import widens every literal to `string`,
 * so it cannot satisfy `Parsed`'s unions; the cast is for that alone.
 */
// dev-lint: allow-untyped-mock a JSON import widens every literal; console/tests/suite/parse.rs pins its shape
export const GOLDEN: Parsed = PARSED_GOLDEN as Parsed;

export const BUSY_BAR = ['mat-progress-bar'];

/** The Material target size, and the size Material's own buttons are not. */
export const THUMB = 48;

/**
 * Let every transform that is still moving finish, before anything measures a
 * box.
 *
 * A box measured mid-animation is not the box the layout claims. Material
 * opens its overlays by scaling them: `_mat-menu-enter` runs a panel from
 * `scale(0.8)` to `scale(1)` over 120ms, and `getBoundingClientRect` maps
 * through that, so every 48px menu item inside is genuinely 38px early in the
 * window and 47.99px in the last frame of it. Both read as a control too small
 * to hit, and neither is: with the CPU throttled, `expectThumbTargets` fails on
 * an open menu every time without this wait and passes every time with it.
 *
 * This is not "wait for the overlay to be ready". Nothing running means it
 * returns at once, and an overlay whose contents are still on their way has
 * nothing running yet — so a check that measures those contents must wait for
 * them to EXIST first and use this only to stop them moving. See [[openParse]],
 * where using it alone read `.raw` as `null` on a page about to be correct.
 *
 * Finite animations only, and this is not a detail. A busy session keeps
 * `mat-progress-bar` cycling forever, so waiting for `getAnimations()` to drain
 * would never return on the page these checks are mostly pointed at. Bounded as
 * well, because an animation that is never going to finish should fail the
 * assertion that follows rather than the 90s test timeout.
 */
export async function settleTransforms(page: Page, bound = 2_000): Promise<void> {
  await page.evaluate(async (ms) => {
    const moving = document.getAnimations().filter((a) => {
      if (a.effect?.getTiming().iterations === Infinity) return false;
      const frames = a.effect instanceof KeyframeEffect ? a.effect.getKeyframes() : [];
      return frames.some((frame) => 'transform' in frame || 'scale' in frame);
    });
    if (moving.length === 0) return;
    // dev-lint: allow-ignored-error `finished` rejects on a cancelled animation, and an overlay torn down while arriving is settled for any reading here
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
export async function expectThumbTargets(page: Page, min = THUMB): Promise<void> {
  await settleTransforms(page);
  const small = await page.evaluate((least) => {
    const missed: { label: string; width: number; height: number; scaledBy?: string }[] = [];
    for (const control of document.querySelectorAll('button, a[href]')) {
      const box = control.getBoundingClientRect();
      // Not rendered at all — a disabled control in a collapsed branch.
      if (box.width === 0 && box.height === 0) continue;
      if (box.height >= least && box.width >= least) continue;
      // Reported to the hundredth, deliberately: rounded, a row that misses by
      // 0.004px is reported as `height: 48`, the size the assertion just refused.
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
 * The one failure class every other check here is blind to. A glyph 3px
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
export async function expectIconsCentred(page: Page, slack = 1): Promise<void> {
  // The same reason as `expectThumbTargets`: two boxes compared while an
  // overlay is still scaling are two boxes nobody has looked at yet.
  await settleTransforms(page);
  const skewed = await page.evaluate((tolerance) => {
    const off: { label: string; dx: number; dy: number }[] = [];
    for (const control of document.querySelectorAll('button, a[href]')) {
      const glyphs = control.querySelectorAll('mat-icon');
      const only = glyphs[0];
      // Only controls that ARE an icon. A button with an icon beside a label
      // places the pair, and neither one belongs in the middle by itself.
      if (
        !only ||
        glyphs.length !== 1 ||
        (control.textContent ?? '').trim() !== only.textContent?.trim()
      )
        continue;
      const box = control.getBoundingClientRect();
      const glyph = only.getBoundingClientRect();
      if (box.width === 0 || glyph.width === 0) continue;
      const dx = glyph.x + glyph.width / 2 - (box.x + box.width / 2);
      const dy = glyph.y + glyph.height / 2 - (box.y + box.height / 2);
      if (Math.abs(dx) <= tolerance && Math.abs(dy) <= tolerance) continue;
      off.push({
        label: only.textContent?.trim() ?? control.className,
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
 * The same blind spot `expectIconsCentred` covers, one row down. A time
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
export async function expectClocksOnTheirLine(page: Page, slack = 2): Promise<void> {
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
      // Tops, not centres. The clock is body-small beside rows that are
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

export const REPOS = [
  '/home/example/Code/health',
  '/home/example/Code/memview',
  '/home/example/Code/nixos-config',
];

/** The session most of these tests open: deep path, long first instruction, still alive. */
export const RUNNING: Summary = {
  id: '6f7c2f11-0000-4000-8000-000000000001',
  dir: '/home/example/Code/health/packages/health-sync-backend/src/decode',
  started: 1785600000,
  alive: true,
  model: 'claude-opus-5[1m]',
  // The chip's effort bars exist only for a session that recorded one.
  effort: 'xhigh',
  // Without this the header's mode icon does not exist in the harness.
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
  working: false,
  interactions: 12,
  cost_usd: 4.2137,
  background: 0,
  running: [],
  waiting: 1,
  unread: 0,
  messages: [],
  asked:
    'Port the remaining matcher gate to Lean and prove it bit-exact against the TypeScript quant twin, then run the golden set and report which journeys moved.',
};

/** The other one on the roster: ended, cheap, and holding no tasks. */
export const ENDED: Summary = {
  id: '6f7c2f11-0000-4000-8000-000000000002',
  dir: '/home/example/Code/memview',
  started: 1785599000,
  alive: false,
  model: 'claude-haiku-4-5-20251001',
  working: false,
  interactions: 3,
  cost_usd: 0.0084,
  background: 0,
  running: [],
  waiting: 0,
  unread: 0,
  messages: [],
  asked: 'check the corpus',
};

/** A roster with the shapes that crowd a narrow screen: a deep path, a long
 *  first instruction, a session that is working and one that has ended. */
export const STATE: Overview = {
  dirs: ['/home/example/Code'],
  repos: REPOS,
  sessions: [RUNNING, ENDED],
  // Who is holding what, keyed by id the way the runner sends it — every holder
  // counted in one sweep, so the row and the ⋮ menu both read the same two
  // numbers without opening anything. The second session keeps no list, which is
  // the ordinary case and the reason a row can say nothing rather than `0/0`.
  // See `console/src/tasks.rs`.
  tasks: {
    sessions: { '6f7c2f11-0000-4000-8000-000000000001': { open: 2, total: 3, stray: 0 } },
    elsewhere: [],
  },
  gists: {},
  drafts: {},
  // A reading in the state it usually arrives in: hours old, its short window
  // long since turned over. See `console/src/usage.rs`.
  usage: {
    host: 'mac-mini',
    age_ms: 4 * 3_600_000,
    five_hour: { pct: 28 },
    seven_day: { pct: 66, resets_in_ms: 54 * 3_600_000 },
    models: [],
  },
};

/**
 * The same roster, long enough that the list cannot fit on a phone.
 *
 * The two-session fixture above is a screen and a half short of the defect.
 * The console drives a dozen sessions at once — that is what it is for — and
 * every existing check passed a list that was quietly painting over the build
 * stamp, because no fixture had ever made the page taller than the viewport.
 */
export const CROWDED = {
  ...STATE,
  sessions: Array.from({ length: 12 }, (_, index) => ({
    ...(index % 2 === 0 ? RUNNING : ENDED),
    id: `6f7c2f11-0000-4000-8000-0000000${String(index + 10).padStart(5, '0')}`,
    dir: REPOS[index % REPOS.length] ?? RUNNING.dir,
  })),
} satisfies Overview;

/** Two fixed moments either side of midnight, so the date row between them is on
 *  screen for every run rather than on the runs that happen to straddle one. */
export const LATE = new Date(2026, 6, 30, 23, 52).getTime();
export const NEXT = new Date(2026, 6, 31, 8, 14).getTime();

/** What a `Read` of a long file comes back as: the runner cuts it, and the page
 *  has to hold the cut without widening — the lines are unwrapped source. */
export const LONG_RESULT = Array.from(
  { length: 30 },
  (_, line) =>
    `   ${line + 1}\texport const quantiseLegCost = (leg: MatcherLeg, budget: number): number =>`,
).join('\n');

/** A transcript with every entry kind, and the arguments that are hardest to
 *  fit: an absolute path, a piped shell command, a failed call, a cut tool
 *  result, and an answer carrying a fenced code block of unbreakable lines. */
export const TRANSCRIPT = [
  { kind: 'started', model: 'claude-opus-5[1m]', cwd: RUNNING.dir, tools: 30, at: LATE },
  // A call whose result was never written, and the boundary that reveals it. The
  // process that would have recorded the answer died first, so the row carries
  // no verdict and no clock — a third state beside the tick and the cross, and
  // one a 412px row has to hold without pushing the tool's own argument out.
  {
    kind: 'tool',
    id: 'toolu_00',
    name: 'Bash',
    does: { kind: 'bash', command: 'nix develop -c home-manager switch --flake .#user' },
    at: LATE,
  },
  // Carries an `at`, as a real `Joined` always does — the runner pushes it through `push()`, which
  // stamps `now()` — so a note row without one tests a shape production never
  // sends. With one, this row is drawn with a clock in the margin, which is what
  // caught an input's styling leaking onto it through an unscoped `.note`.
  { kind: 'joined', earlier: 1, from: 0, restarted: false, at: LATE },
  {
    kind: 'prompt',
    text: 'Port the remaining matcher gate to Lean and prove it bit-exact against the TypeScript quant twin.',
    at: LATE,
  },
  {
    kind: 'tool',
    id: 'toolu_01',
    name: 'Read',
    does: {
      kind: 'other',
      shown: '/home/example/Code/health/packages/health-sync-backend/src/decode/matcher-gate.ts',
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
    does: {
      kind: 'bash',
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
    // In colour, because that is how it arrives. `cargo`, `vitest` and
    // `eslint` all write SGR, and with nothing reading it the ESC byte is
    // invisible while the codes are not — the phone showed `[2m…[32m22 passed`.
    // The fixture carries the escapes so the harness renders what a real result
    // renders.
    detail:
      '\x1b[1;31merror\x1b[0m: unknown flag \x1b[33m--serve\x1b[39m\n' +
      '\x1b[2mnote: run with --help for a list\x1b[22m\n' +
      '\x1b[90m Test Files \x1b[39m \x1b[1m\x1b[32m22 passed\x1b[39m\x1b[22m\x1b[90m (22)\x1b[39m\n' +
      'exit status 2',
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
    does: { kind: 'bash', command: 'nix develop -c lake build --verbose 2>&1 | tee /tmp/lean.log' },
    at: NEXT,
  },
] satisfies Timed[];

/**
 * A transcript ending in a question of the kind a person answers.
 *
 * Deliberately the awkward one: four options, descriptions that are whole
 * sentences, and a `multiSelect` question after it — so the page has to hold a
 * column of two-line controls AND a send button on a 412px screen. A question
 * with two one-word options would prove nothing.
 */
export const QUESTION_TRANSCRIPT = [
  { kind: 'started', model: 'claude-opus-5[1m]', cwd: '/home/example/Code/memview', tools: 14 },
  { kind: 'text', text: 'Two things before I build it.', at: NEXT },
  {
    kind: 'ask',
    id: 'c9f0a1b2-0000-4000-8000-00000000000a',
    tool: 'AskUserQuestion',
    does: {
      kind: 'question',
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
] satisfies Timed[];

/** Mock every backend call. Catch-all FIRST — Playwright runs handlers
 *  last-registered-first. The event stream is served as a complete SSE body, so
 *  the transcript renders without a live runner. */
/**
 * Open the folded run of tool calls, so the calls themselves are on the page.
 *
 * A run of two or more calls is folded by default — see `blocks()` in
 * transcript.ts — so a check about what a tool row looks like has to open it
 * first. That the fixture folds at all is the subject of its own test below.
 */
export async function openTools(page: Page): Promise<void> {
  await page.locator('.entry.tools .run').first().click();
}

/**
 * The corpus survey, at the widths that stress a 412px strip.
 *
 * The longest real shape label is here on purpose. "run a program on
 * another machine (no shell)" is 42 characters of prose beside a right-aligned
 * seven-digit count, which is the one row that can collide.
 *
 * Every field the page draws, not only the ones it asserts. This carried
 * four while it fed a five-bar strip; the page reads a dozen more, and Angular's
 * `| number` renders `undefined` as EMPTY rather than failing — so the Files
 * figures, the work queue and the stamp were all blank while every assertion
 * passed. Caught by looking at the render. A missing field here is invisible
 * unless something asserts the number it produces, which is why the walk below
 * now names one from each section.
 */
export const READING = {
  commands: 2446441,
  understood: 99.17950197858849,
  opaque: 4.579426844363917,
  reads: 411965,
  writes: 76770,
  distinct_paths: 37747,
  always: 354612,
  on_success: 73664,
  sometimes: 60451,
  certain: 425085,
  table_reads: 2747,
  table_writes: 264,
  distinct_tables: 151,
  calls: 294504,
  unparsed: 102,
  unread: [
    { name: 'k3s', n: 1215 },
    { name: 'arp', n: 167 },
  ],
  local: 2475,
  local_names: [
    { name: 'probe', n: 418 },
    { name: 'render', n: 162 },
  ],
  doing: [
    { name: 'nothing with files', n: 1072884 },
    { name: 'read', n: 432923 },
    { name: 'reach another machine (ssh, kubectl exec)', n: 54780 },
    { name: 'run a program on another machine (no shell)', n: 3096 },
    { name: 'not understood', n: 20073 },
  ],
  corpus_at: 1787441804,
  unrolled: 969629,
  handled: 2426368,
  unhandled: 20073,
  from_a_variable: 307,
  unnamed: 23455,
  unnamed_by_word: 9475,
  unnamed_bounded: 942,
  unnamed_located: 611,
  unnamed_computed: 11139,
  unnamed_python_set: 2719,
  unnamed_javascript: 489,
  refused_here: 921,
  tables: [
    { name: 'report', reads: 331, writes: 0 },
    // A qualified name is a distinct subject from its bare form, and both are
    // real — `health.hrv_intraday` and `hrv_intraday` are counted apart.
    { name: 'health.hrv_intraday', reads: 64, writes: 0 },
    { name: 'location_equipment_option', reads: 0, writes: 14 },
  ],
  sql: [
    { name: 'select', n: 2432 },
    { name: 'drop', n: 76 },
  ],
  renames: 578,
  busiest: [
    {
      name: '/home/example/Code/memview/frontend/projects/console-web/e2e/ui-pages.spec.ts',
      reads: 5414,
      writes: 33,
    },
    { name: 'MEMORY.md', reads: 381, writes: 262 },
  ],
  writers: [{ name: 'python', reads: 21276, writes: 12772 }],
  hosts: [{ name: 'odin', reads: 14863, writes: 974 }],
  variable_names: [
    { name: '$ADB', n: 77 },
    { name: '$BIN', n: 39 },
  ],
  opaque_words: [
    { name: '$f', n: 2378 },
    { name: '$(find InstantUpload -name "$f" | head -1)', n: 96 },
  ],
} satisfies CorpusRead;

/**
 * Route a session's event stream the way the runner serves it. A fresh
 * connection starts with `reset`, so the page empties what it holds before the
 * history arrives; a RESUME gets only the events after the last one it saw, as
 * the runner's backlog does. Two ways to resume, both the runner's:
 * the browser's own reconnect sends `Last-Event-ID`, and the app's reopen asks
 * for `?after=N`. The mocks had none of it: a finite body ends the stream, the
 * page connects again about 3s later, and the history was appended to itself —
 * a sent picture drawn twice, 13 of 60 runs under load. Not "the
 * first connection wins": under load the page can open a fresh stream before
 * the view's own. A test about reconnecting routes the stream itself.
 */
export async function routeStream(
  page: Page,
  handler: (route: Route) => Promise<void>,
): Promise<void> {
  // A runner's stream stays open; a mock's body ends, and the browser reconnects
  // after `retry`. Ten minutes outlasts any test, so the stream reads as open and
  // quiet, which is what an idle session's is. Events are numbered as the
  // runner's are, and a resume gets those after the one it names.
  const served = (body: string, after: number): string =>
    'retry: 600000\n\n' +
    (after > 0 ? '' : 'event: reset\ndata: this stream starts again from the beginning\n\n') +
    body
      .split('\n\n')
      .map((chunk, n) => ({ chunk, id: n + 1 }))
      .filter(({ chunk, id }) => !chunk.startsWith('data:') || id > after)
      .map(({ chunk, id }) => (chunk.startsWith('data:') ? `id: ${id}\n${chunk}` : chunk))
      .join('\n\n');
  await page.route('**/api/sessions/*/events', async (r) => {
    const asked = r.request();
    // `allHeaders`, not `headers`: the synchronous list leaves some out.
    const after = Math.max(
      Number((await asked.allHeaders())['last-event-id'] ?? 0) || 0,
      Number(new URL(asked.url()).searchParams.get('after') ?? 0) || 0,
    );
    const route = new Proxy(r, {
      get(target, key) {
        if (key === 'fulfill') {
          return (options: Parameters<Route['fulfill']>[0]) =>
            target.fulfill(
              typeof options?.body === 'string'
                ? { ...options, body: served(options.body, after) }
                : options,
            );
        }
        const value: unknown = Reflect.get(target, key, target);
        return typeof value === 'function'
          ? (value as (...args: unknown[]) => unknown).bind(target)
          : value;
      },
    });
    return handler(route);
  });
}

export async function mockRunner(page: Page): Promise<void> {
  await page.route('**/api/**', (r) => r.fulfill({ status: 204, body: '' }));
  // What every page reads on load, answered as the runner would with nothing to
  // say: the catch-all once answered these `[]` without anybody choosing to.
  await page.route('**/api/past', (r) => r.fulfill({ json: [] satisfies Conversation[] }));
  await page.route('**/api/sessions/*/tasks', (r) => r.fulfill({ json: [] satisfies Task[] }));
  await page.route('**/api/sessions/*/edits', (r) =>
    r.fulfill({ json: { edited: [], diverged: [] } satisfies EditRecord }),
  );
  await page.route('**/api/state', (r) => r.fulfill({ json: STATE }));
  await page.route('**/api/reading', (r) => r.fulfill({ json: READING }));
  await routeStream(page, (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: TRANSCRIPT.map((event) => `data: ${JSON.stringify(event)}\n\n`).join(''),
    }),
  );
}

/**
 * Nothing pinned may sit on top of anything else pinned.
 *
 * Not covered by the text-overlap check, which is why this exists as its
 * own assertion. That one compares text nodes, and the composer's biggest
 * surface is an empty `textarea` with no text in it at all — so a footer
 * rendering straight through the text box was invisible to it, while being the
 * first thing anybody saw on the phone. Boxes, not words.
 */
export async function expectNoPinnedOverlap(page: Page): Promise<void> {
  const clashes = await page.evaluate(() => {
    const boxes = ['.composer', '.build', 'ui-scaffold'].map((sel) => ({
      sel,
      box: document.querySelector(sel)?.getBoundingClientRect(),
    }));
    const bad: string[] = [];
    for (let i = 0; i < boxes.length; i++) {
      for (let j = i + 1; j < boxes.length; j++) {
        const one = boxes[i];
        const other = boxes[j];
        const a = one?.box;
        const b = other?.box;
        if (!one || !other || !a || !b) continue;
        const over =
          Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top) > 1 &&
          Math.min(a.right, b.right) - Math.max(a.left, b.left) > 1;
        if (over) bad.push(`${one.sel} over ${other.sel}`);
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
export async function expectComposerFillsTheWidth(page: Page): Promise<void> {
  const width = await page.evaluate(() => {
    const composer = document.querySelector('.composer')?.getBoundingClientRect();
    const field = document.querySelector('.composer .box')?.getBoundingClientRect();
    return composer && field
      ? { composer: composer.width, field: field.width, page: window.innerWidth }
      : null;
  });
  expect(width, 'no composer on this page').not.toBeNull();
  // The composer reaches both edges; the text box takes what is left after the
  // one control beside it. 412px of screen less 32 of padding, 48 of button and
  // 8 of gap leaves 324: a second control on the row would take it below this.
  expect(width!.field).toBeGreaterThan(width!.page * 0.75);
}

/**
 * The button beside the box (send, or the picture) sits on the same line as it.
 *
 * Not a cosmetic assertion. A `mat-form-field` reserves a line of hint/error
 * space below its input by default — `.mat-mdc-form-field-bottom-align::before`
 * in Material's own CSS — so a row aligned to `flex-end` aligns its other
 * controls to the bottom of a box whose last line is invisible. The button then
 * sits a full subscript line low, and every existing check passes it: it is not
 * clipped, not small, not overflowing, and does not overlap anything.
 */
export async function expectSendAlignsWithTheBox(page: Page): Promise<void> {
  const rows = await page.evaluate(() => {
    // The OUTLINE's box, not the textarea's: the field pads its own text, so a
    // textarea bottom is inset from the visible edge the eye lines up against.
    const input = document.querySelector('.composer .box')?.getBoundingClientRect();
    const send = document.querySelector('.composer .slot')?.getBoundingClientRect();
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
 * The failure class none of the others can see. Text a few pixels off its
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
 *
 * Roboto has to be LOADED first, since the cap height above is its own. Measured
 * earlier, the row is in whatever face the machine has: a fallback font can be
 * 1.1px out on one run and pass the next.
 */
export async function expectOneLine(page: Page, rowSel: string, tol = 1): Promise<void> {
  await expectIconFontLoaded(page, 'Roboto');
  await page.evaluate(() => document.fonts.ready);
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
 * Nothing in the page paints over `selector` — the stamp at the foot of the
 * shell, or the button pinned above it.
 *
 * Two claims, one implementation. A second copy of the clipping walk below
 * is a second answer to keep in step, and the two targets differ in nothing but
 * which box is the target: `.add` is skipped when it IS the target, and a caller
 * scrolls first when the overlap only happens at the foot of a list.
 *
 * A different failure from `expectNoPinnedOverlap`, which compares the
 * pinned regions with each other and never looks at the content. The shell is a
 * `100dvh` column of toolbar, page and stamp, and `.page` is deliberately
 * shrinkable — `min-height: 0` is what lets a flex item be shorter than what is
 * inside it. Shorter, with `overflow: visible`, means the content is neither
 * clipped nor scrolled: it spills out of the box and paints over the next row.
 * So the check has to be against what the page actually holds.
 */
export async function expectNothingPaintsOver(page: Page, selector: string): Promise<void> {
  const clashes = await page.evaluate((selector) => {
    const target = document.querySelector(selector)?.getBoundingClientRect();
    if (!target) return [`no ${selector} on this page`];

    // What is painted, not what is laid out. A card scrolled halfway out
    // of a scrolling region still reports its whole box from
    // `getBoundingClientRect` — the clipping is done by the ancestor, and it is
    // the difference between the defect and the fix: the same card, at the same
    // coordinates, is either over the stamp or cut off above it depending on one
    // `overflow` declaration further up. So each box is intersected with every
    // ancestor that actually clips.
    const painted = (node: Element): DOMRect => {
      let box = node.getBoundingClientRect();
      // A fixed element is not clipped by its ancestors' overflow — it is
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
    // `.add` is here because it is the one thing on this page positioned
    // against the viewport rather than laid out in the column — which is exactly
    // the way to end up on top of the stamp, and the only way this check can be
    // wrong by omission. A selector naming an element that no longer exists is a
    // check quietly covering less than it reads as covering.
    for (const card of document.querySelectorAll('.page .session, .page .past, .page .add')) {
      if (card.matches(selector)) continue;
      const box = painted(card);
      const over =
        Math.min(box.bottom, target.bottom) - Math.max(box.top, target.top) > 1 &&
        Math.min(box.right, target.right) - Math.max(box.left, target.left) > 1;
      if (over) {
        bad.push(
          `${card.className.split(' ')[0]} [${Math.round(box.top)}–${Math.round(box.bottom)}]` +
            ` over ${selector} [${Math.round(target.top)}–${Math.round(target.bottom)}]`,
        );
      }
    }
    return bad;
  }, selector);
  expect(clashes, `the page paints over ${selector}`).toEqual([]);
}

/**
 * Hand the test the stream, so it decides when a message arrives.
 *
 * The mocked SSE body cannot answer this question. `route.fulfill` serves
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
export async function handControlOfTheStream(page: Page): Promise<void> {
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
      // The boundary first, exactly as the runner sends it. Every real
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
export async function mutter(page: Page, event: unknown, seq: number): Promise<boolean> {
  return page.evaluate(
    ([one, at]) => (window as unknown as { __say(e: unknown, n: number): boolean }).__say(one, at),
    [event, seq] as [unknown, number],
  );
}

/** Say something on the stream and let the view lay it out and react. */
export async function say(page: Page, event: unknown, seq: number): Promise<void> {
  const delivered = await mutter(page, event, seq);
  expect(delivered, 'nothing was listening to the stream').toBe(true);
  // Two frames: `session-view.ts` follows in a `requestAnimationFrame` after the
  // entries change, so one frame is the render and the second is the reaction.
  await page.evaluate(
    () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))),
  );
}

/** How far the transcript is from its own end, in pixels. */
export async function distanceFromTheEnd(page: Page): Promise<number> {
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
 * Trusted touch events through CDP, not `dispatchEvent`. A synthetic
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
export async function thumb(page: Page, by: number, during?: () => Promise<void>): Promise<void> {
  const box = (await page.locator('.transcript').boundingBox())!;
  const x = Math.round(box.x + box.width / 2);
  const y = Math.round(box.y + box.height / 2);
  const cdp = await page.context().newCDPSession(page);
  const finger = { id: 1, radiusX: 8, radiusY: 8, force: 1 };
  await cdp.send('Input.dispatchTouchEvent', {
    type: 'touchStart',
    touchPoints: [{ ...finger, x, y }],
  });
  // Paced like a hand, not dispatched in a burst. Sent back to back, the
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
  // Still before it lifts, or this is a flick and not a thumb. Releasing
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

/**
 * Landmarks as the runner sends them, and shaped like the hard cases.
 *
 * The long one is not padding. A first line is often a pasted path or a
 * command with no spaces in it, which is exactly what breaks a single-line row:
 * `overflow-wrap: anywhere` collapses the column to its longest word
 * (DL-CSS-ANYWHERE) and a missing `min-width: 0` pushes the icon off the screen.
 * Two days, so the grouping is drawn rather than assumed.
 */
/**
 * A landmark's moment, as a wall-clock time on a day counted back from today.
 *
 * Anchored to the run's own clock, and it has to be. `byDay` names a day
 * `Today` or `Yesterday` by comparing `toDateString()` against `Date.now()`, so a
 * fixture pinned to absolute dates passes on the day it is written and fails
 * every day after — which is what happened. Local rather than UTC for the same
 * reason `byDay` is local: the two must
 * agree about which calendar day a moment falls on, or the test asserts against a
 * grouping the app would never produce.
 */
export function daysBack(days: number, hour: number, minute: number): number {
  const when = new Date(Date.now() - days * 86_400_000);
  when.setHours(hour, minute, 0, 0);
  return when.getTime();
}

export const LANDMARKS = [
  {
    at: 1024,
    when: daysBack(1, 9, 14),
    kind: 'prompt',
    text: 'run the decoder against /home/example/Code/health/packages/health-sync-backend/src/decode/fixtures/2026-07-31-overnight.json and say what it makes of the gaps',
  },
  { at: 4096, when: daysBack(1, 11, 2), kind: 'command', text: 'compact' },
  { at: 8192, when: daysBack(1, 11, 2), kind: 'compacted', text: '' },
  { at: 16384, when: daysBack(0, 8, 30), kind: 'shown', text: 'what is wrong with this?' },
  { at: 32768, when: daysBack(0, 9, 5), kind: 'prompt', text: 'that is the one, thanks' },
] satisfies Landmark[];

/**
 * A real 2×4 PNG, on disk beside this spec.
 *
 * A file rather than bytes in the source, and not for tidiness. PNG bytes
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
export function tinyPng(): string {
  // Anchored to the project's own test directory rather than to the process's
  // working directory, which is the repo's frontend/ when the gate runs this and
  // something else when it is run by hand. `test.info()` is Playwright's own and
  // needs no Node types.
  return `${test.info().project.testDir}/fixtures/tiny.png`;
}

/**
 * A run of calls with a permission question standing on the newest one.
 *
 * The CLI announces the call and then asks about it, carrying the same
 * `tool_use` id on both. Two calls before it, so the fold has something to fold
 * and a card between them would break the run.
 */
export const DECIDING = [
  { kind: 'started', model: 'claude-opus-5[1m]', cwd: '/home/example/Code/memview', tools: 14 },
  {
    kind: 'tool',
    id: 'toolu_a1',
    name: 'Bash',
    does: { kind: 'bash', command: 'git status' },
    at: NEXT,
  },
  { kind: 'tool_result', id: 'toolu_a1', ok: true, detail: 'nothing to commit' },
  {
    kind: 'tool',
    id: 'toolu_a2',
    name: 'Bash',
    does: { kind: 'bash', command: 'git diff --stat' },
    at: NEXT,
  },
  { kind: 'tool_result', id: 'toolu_a2', ok: true, detail: '2 files changed' },
  {
    kind: 'tool',
    id: 'toolu_a3',
    name: 'Write',
    does: { kind: 'other', shown: '/home/example/Code/memview/notes.md' },
    at: NEXT,
  },
  {
    kind: 'ask',
    id: 'c8471a53-2b00-4a5e-a8d3-610f6d8c6b07',
    call: 'toolu_a3',
    tool: 'Write',
    title: 'Claude wants to write /home/example/Code/memview/notes.md',
    does: { kind: 'other', shown: '/home/example/Code/memview/notes.md' },
    at: NEXT,
  },
] satisfies Timed[];

/** Serve the question transcript, and record what any decision sends. */
export async function mockQuestion(page: Page): Promise<() => Record<string, unknown> | undefined> {
  let sent: Record<string, unknown> | undefined;
  await mockRunner(page);
  await routeStream(page, (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: QUESTION_TRANSCRIPT.map((event) => `data: ${JSON.stringify(event)}\n\n`).join(''),
    }),
  );
  await page.route('**/api/sessions/*/decide', (r) => {
    sent = r.request().postDataJSON() as Record<string, unknown>;
    return r.fulfill({ json: RUNNING });
  });
  return () => sent;
}

/** A paragraph tall enough that not following is unmistakable — well past the
 *  120px of slack `session-view.ts` allows before it calls a reader "left
 *  behind", and past a phone screen too. */
export const LONG_ANSWER = Array.from(
  { length: 40 },
  (_, line) => `Line ${line + 1} of an answer long enough to move the end of the transcript.`,
).join('\n\n');

/** Every state the list can be in at once, in an order that is nobody's idea of
 *  correct — so the page has to be doing the sorting rather than the fixture. */
export const MIXED = {
  ...STATE,
  sessions: [
    { ...ENDED, id: 'aaaa0000-0000-4000-8000-000000000001', name: 'finished' },
    {
      ...RUNNING,
      id: 'aaaa0000-0000-4000-8000-000000000002',
      name: 'idle-one',
      busy: undefined,
      waiting: 0,
    },
    {
      ...RUNNING,
      id: 'aaaa0000-0000-4000-8000-000000000003',
      name: 'blocked',
      busy: undefined,
      waiting: 1,
    },
    { ...RUNNING, id: 'aaaa0000-0000-4000-8000-000000000004', name: 'working' },
  ],
} satisfies Overview;

/** Conversations on disk, one of them held by something the console cannot see. */
export const ON_DISK = [
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
] satisfies Conversation[];

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
 * `done` / `open` / `doing`, not `completed` / `pending` / `in_progress`.
 * The second set is the built-in task tool's vocabulary, which this read from
 * disk until the lists moved to the service. This fixture kept saying
 * `completed` afterwards, so the sheet — which hides `status === 'done'` — put
 * finished work on a screen that claims not to show any, and the spec that would
 * have caught it is not run by the gate.
 *
 * `active_form` and `blocked_by` are gone with them: the service has neither
 * field, so nothing here may invent one. See `console/src/tasks.rs`.
 */
export const TASKS = [
  // In the service's order, ranks and all — `repo::list` is the only sort
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
    overdue: false,
    blocked_on: [],
    blocked: false,
  },
  {
    id: '2',
    subject: 'bless the golden set after the oracle moved',
    status: 'done',
    detailed: true,
    overdue: false,
    blocked_on: [],
    blocked: false,
  },
  {
    id: '100',
    subject: 'write the rule up',
    status: 'open',
    detailed: false,
    overdue: false,
    blocked_on: [],
    blocked: false,
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
    overdue: false,
    blocked_on: [],
    blocked: false,
  },
  {
    id: '101',
    subject: 'port the matcher gate',
    status: 'doing',
    detailed: true,
    // A deadline that is NOT overdue, beside one that is. The pair is the
    // point: a date that has not been missed is a fact about the task, not a
    // problem with it, and the render has to keep the two apart. The service may
    // leave `overdue` out; the console reads that as false and sends it so.
    due: '2026-09-01',
    overdue: false,
    blocked_on: [],
    blocked: false,
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
    overdue: false,
    blocked: false,
  },
  // The fourth state: `dropped` is closed without ever being done, so a filter
  // on `status !== 'done'` would stand it among the open ones.
  {
    id: '54',
    subject: 'move the per-session repo claim into the service',
    status: 'dropped',
    detailed: false,
    overdue: false,
    blocked_on: [],
    blocked: false,
  },
] satisfies Task[];

/** A session the runner has finished reading: it has a name, a model id and a
 *  permission mode, which is what the toolbar and the sheet divide between them. */
export const NAMED = {
  ...STATE,
  sessions: [
    {
      ...RUNNING,
      name: 'health',
      // Deliberately NOT `health`. The two names diverge the moment a
      // conversation is renamed, because only a spawn writes the one its peers
      // use — so the interesting render is the one carrying the note, and a
      // fixture where they agreed would draw a row this suite never sees.
      peer_name: 'code-a7',
      mode: 'bypassPermissions',
      // The two sizes: what the model still holds, and what has been said. 62 MB
      // of history under a context two thirds full is a conversation that has
      // forgotten most of itself.
      context: 640_000,
      window: 1_000_000,
      bytes: 65_011_712,
    },
    ENDED,
  ],
  // What it is about, keyed by conversation the way the runner sends it. Long
  // enough that the card clamps it, which is the case the sheet exists for.
  gists: {
    '6f7c2f11-0000-4000-8000-000000000001': {
      text:
        'porting the last of the matcher gate to Lean, proving it bit-exact against the ' +
        'TypeScript twin, and running the golden set to see which journeys moved',
      name: null,
      at: 1785600000000,
      bytes: 180_000_000,
    },
  },
} satisfies Overview;

/**
 * Give the page a wake lock that always works, before anything runs.
 *
 * Stubbed rather than used. Headless Chromium has no display to keep on
 * and refuses the real request, which the app correctly treats as a refusal and
 * puts the button back — so a test of the wiring would fail on the one thing it
 * is not testing. What Android actually does with a lock is a phone's answer,
 * not a harness's; this asserts the console asks, draws the answer, and offers
 * the control in both places.
 */
export async function withWakeLock(page: Page): Promise<void> {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'wakeLock', {
      configurable: true,
      value: {
        request: () => Promise.resolve({ released: false, release: () => Promise.resolve() }),
      },
    });
  });
}

/** Where the toolbar's leading glyph starts — the terminal mark, or the arrow. */
export async function leadingGlyph(page: Page): Promise<number> {
  return page.evaluate(
    () => document.querySelector('ui-scaffold mat-icon')!.getBoundingClientRect().left,
  );
}

/**
 * The runner's real answer for the transcript fixture's own failed command.
 *
 * Read from a golden the Rust test `the_shape_the_phone_is_drawn_from` writes,
 * never copied: a hand-written stub agrees with whatever it was last told.
 *
 * `console/tests/suite/parse.rs` now compares its own output against
 * `parsed.fixture.json` and fails with instructions; `BLESS=1` updates it, and
 * the change lands in the same commit as the reader change that caused it.
 *
 * The command is the transcript fixture's own, and it FAILED — which is why it
 * is worth drawing: everything after the `&&` parses, classifies and names a
 * real file, and none of it is certain.
 */
export const PARSED: Parsed = GOLDEN;

/** As [[mockRunner]], and answering the parse the sheet asks for. */
export async function mockParse(page: Page, answer: Parsed = PARSED): Promise<void> {
  await page.route('**/api/sessions/*/parse', (r) => r.fulfill({ json: answer }));
}

/**
 * The sheet, and the scope every layout check on it is given.
 *
 * An overlay legitimately covers the page it is over. Unscoped, the
 * overlap check compares this sheet's text against the transcript still rendered
 * behind it and reports seventeen collisions that are the whole point of a
 * bottom sheet. Same reasoning, and the same argument, as the details sheet.
 */
export const SHEET = '.session-sheet';

/**
 * Open the parse of the fixture's failed shell command, with the sheet settled.
 *
 * Waits for the animation, not for the element. A Material bottom sheet
 * slides up, so a locator is visible — and measurable, and wrong — while it is
 * still 300px below where it will come to rest. That is not flakiness to retry
 * around: it is a box that has not finished moving, and the fix is to wait for
 * the thing that is moving it.
 */
export async function openParse(page: Page): Promise<void> {
  await openTools(page);
  await page.getByRole('button', { name: /verified_cli/ }).click();
  const sheet = page.locator(SHEET);
  await sheet.waitFor();
  // Wait for the ANSWER, not for the sheet and not for `.raw`. The sheet
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
  await page.locator(`${SHEET} .summary, ${SHEET} .unread, ${SHEET} .trouble`).first().waitFor();
  // `settleTransforms`, and the two earlier waits here were both wrong.
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
  // where it comes to rest.
  await settleTransforms(page);
}

/** The `transform` the viewer has put on the picture, read back as numbers. */
export async function placed(page: Page): Promise<{ scale: number; x: number; y: number }> {
  return page.locator('ui-picture-sheet img').evaluate((img) => {
    const matrix = new DOMMatrixReadOnly(getComputedStyle(img).transform);
    return { scale: matrix.a, x: matrix.e, y: matrix.f };
  });
}

/** Open the viewer on a render big enough to have somewhere to move to. */
export async function opened(page: Page): Promise<void> {
  await mockRunner(page);
  await routeStream(page, (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: (
        [
          { kind: 'started', model: 'claude-opus-5[1m]', cwd: '/home/example/Code', tools: 14 },
          { kind: 'text', text: '![the room](/home/example/peek/room.png)', at: NEXT },
        ] satisfies Timed[]
      )
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.route('**/api/picture*', (r) =>
    // Not `tiny.png`. At 2×4 it is smaller than the phone even at the
    // closest magnification, so there is nowhere to drag it to and the pan
    // assertion reads zero against a view that is behaving perfectly. This one
    // is 800×1000 — see `fixtures/make-render.mjs` for what it is and why.
    r.fulfill({
      path: `${test.info().project.testDir}/fixtures/render.png`,
      contentType: 'image/png',
    }),
  );
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('a.picture-link').click();
  await page.locator('ui-picture-sheet img').waitFor();
  await settleTransforms(page);
}

export { expect, test, type Locator, type Page, type Route };
export { first, last, nth };
export {
  expectBackClosesOverlay,
  expectCleanLayout,
  expectIconFontLoaded,
  expectNoClippedText,
  expectNoHorizontalOverflow,
  expectUpInTheBar,
  expectViewportIsPhone,
};
export type {
  Call,
  Commands,
  Conversation,
  CorpusRead,
  Described,
  EditRecord,
  Landmark,
  LoginCode,
  LoginResult,
  LoginStarted,
  Message,
  Overview,
  Earlier,
  Parsed,
  Refusal,
  Returned,
  Run,
  SudoAnswer,
  Summary,
  Stretched,
  Task,
  Timed,
};
export { PARSED_GOLDEN };
