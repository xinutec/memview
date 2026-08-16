import { expect, test, type Page } from '@playwright/test';
// The fleet-shared harness, published as @xinutec/ui-harness (source repo
// ~/Code/ui-harness). Ships compiled JS, so it loads straight from node_modules.
import {
  expectIconFontLoaded,
  expectNoHorizontalOverflow,
  expectNoTextOverlaps,
  expectCanvasLegible,
  expectViewportIsPhone,
} from '@xinutec/ui-harness';

/**
 * L2 phone-width layout harness for memview. Render the real screens at a Pixel
 * viewport with the backend mocked and BUSY data, and assert the failure classes
 * that read fine in source and only show on a real phone:
 *   1. no two pieces of rendered text collide, and
 *   2. nothing spills past the right edge.
 *
 * The at-risk spots here are specific to this app: memory names are long
 * unbreakable snake_case tokens (`project_health_verified_core_lean`), the
 * index is dense `·`-separated link runs, and memory bodies carry fenced code
 * and tables that must scroll inside themselves rather than widening the page.
 */
test.use({ serviceWorkers: 'block' });

const ME = { user_id: 'pippijn', display_name: 'Pippijn', shared: false, auth_enabled: true };

/** The index as the backend renders it: dense interpunct-separated link runs
 *  with long slugs — the real MEMORY.md shape. */
const INDEX = {
  html: `<h1>Memory index</h1>
<p>Teasers only — OPEN the file before citing.</p>
<h2>Infrastructure &amp; data services</h2>
<ul><li><a href="/m/project_health_verified_core_lean">Lean verified core: matcher ported + Viterbi-argmax flagship wired into prod</a> · <a href="/m/reference_nixos_2605_dbus_broker_wedge">NixOS 26.05 dbus-broker wedge</a> · <a href="/m/project_fleet_firewall_hardening">firewall hardening (kubelet pending)</a> · <a href="/m/reference_launchd_tcc_external_volume">launchd + /Volumes/Backup: spawn exit 78</a></li></ul>`,
  count: 284,
};

/** One row per `mtype`, so every filter chip has something to show. These are
 *  real index rows — the layout only gets stressed honestly by real slug
 *  lengths and real teaser widths. Keep them to the *technical* end of the
 *  corpus: this file is committed, and a `user_`/`feedback_` teaser about the
 *  person rather than the work has no business travelling with the source. */
const MEMORIES = [
  {
    name: 'project_health_verified_core_lean',
    description: 'Lean 4 port of the health matcher — bit-exact against the TypeScript quant twin',
    mtype: 'project',
    modified: '2026-07-20T09:00:00Z',
  },
  {
    name: 'reference_launchd_tcc_external_volume',
    description:
      'launchd jobs on /Volumes/Backup die with spawn exit 78; the child exec HANGS in dyld',
    mtype: 'reference',
    modified: '2026-07-11T09:00:00Z',
  },
  {
    name: 'feedback_no_magic_strings_use_upstream_taxonomy',
    description:
      "Don't hard-code a magic string from another service; use/extend its taxonomy at the source",
    mtype: 'feedback',
    modified: '2026-07-02T09:00:00Z',
  },
  {
    name: 'user_cycling',
    description: '"cycling" from the classifier is a misclassification',
    mtype: 'user',
    modified: '2026-06-28T09:00:00Z',
  },
];

/** A memory page with everything that can crowd the column: a long slug title,
 *  a long description, prose with inline code, a fenced block, a table, and
 *  both link panels. */
const MEMORY_PAGE = {
  ...MEMORIES[0],
  html: `<p>The <strong>verified core</strong> is a Lean 4 port of the walk matcher, proved
bit-exact against the BigInt quant twin. Run it with <code>LEAN_PASSES=1</code>.</p>
<pre><code>nix develop -c lake build &amp;&amp; ./verified_cli match --serve --timeout 30000ms</code></pre>
<table><thead><tr><th>pass</th><th>tenants</th><th>status</th></tr></thead>
<tbody><tr><td>rejectSpikes</td><td>5</td><td>serving</td></tr></tbody></table>
<blockquote><p>739 verified calls, golden byte-identical under on.</p></blockquote>`,
  backlinks: [MEMORIES[1], MEMORIES[2]],
  outlinks: [MEMORIES[2], MEMORIES[3]],
  dangling: ['project_lean_matcher_flip_soak'],
};

const SEARCH = {
  hits: [
    {
      ...MEMORIES[0],
      snippet:
        '…proved bit-exact against the BigInt quant twin; the flip gate records accepted deltas rather than silently breaking golden byte-identity…',
    },
    {
      ...MEMORIES[1],
      snippet:
        '…launchd + /Volumes/Backup: spawn exit 78; the child exec HANGS in dyld before main…',
    },
  ],
};

/**
 * Agents at the sizes the real artefact reaches, because the totals line is a
 * run of numbers and words that only collides once the numbers are wide. Taken
 * from the live figures: five digits of shell reads, four of maybes, and the
 * whole provenance phrase after them. The second agent carries no maybes at all,
 * so both branches of the template are on screen at once.
 */
const AGENTS = {
  generated: '2026-08-07T12:00:00Z',
  renames: {},
  commits: 3120,
  unattributed: 402,
  agents: [
    {
      name: 'health',
      transcripts: 41,
      delegated: 18,
      sessions: ['a'],
      paths: { 'health/src/geo/annotate-road-matches.ts': { reads: 4820, edits: 3110 } },
      shell_paths: {
        'health/src/geo/velocity.ts': {
          reads: 21689,
          edits: 4114,
          maybe_reads: 1942,
          maybe_edits: 203,
        },
      },
      remote_paths: { 'odin:/etc/nixos/configuration.nix': { reads: 312, edits: 44 } },
      commit_lines: {
        'health/src/geo/velocity.ts': { added: 38104, deleted: 12960, commits: 214 },
      },
      commits: 214,
      reads: { health: 4820 },
      writes: { health: 3110 },
      memories: { project_health_verified_core_lean: { reads: 96, edits: 41 } },
      recent_reads: { health: 12 },
      recent_writes: { health: 9 },
      first: '2026-06-01T00:00:00Z',
      last: '2026-08-07T00:00:00Z',
    },
    {
      name: 'utterance',
      transcripts: 6,
      delegated: 0,
      sessions: ['b'],
      paths: { 'utterance/src/voice/tonnetz.ts': { reads: 210, edits: 88 } },
      shell_paths: {},
      remote_paths: {},
      commit_lines: {},
      commits: 0,
      reads: { utterance: 210 },
      writes: { utterance: 88 },
      memories: { project_utterance_chord_dwell: { reads: 14, edits: 2 } },
      recent_reads: { utterance: 3 },
      recent_writes: { utterance: 2 },
      first: '2026-07-19T00:00:00Z',
      last: '2026-08-05T00:00:00Z',
    },
  ],
};

/** A graph shaped like the real one: three groups that link densely inside
 *  themselves and thinly across, so the clustering has something to find and the
 *  legend has more than one row. Long slugs throughout — a cluster is named
 *  after its most-connected member, so the legend carries the same unbreakable
 *  snake_case tokens the rest of the app does. */
const GRAPH_SECTIONS = [
  'Infrastructure & data services',
  'Rules — deploy & infra ops',
  'health-sync',
];
const GRAPH = {
  sections: GRAPH_SECTIONS,
  nodes: Array.from({ length: 12 }, (_, i) => ({
    name: `project_health_verified_core_lean_${i}`,
    description: 'Lean 4 port of the health matcher — bit-exact against the quant twin',
    mtype: i % 2 === 0 ? 'project' : 'feedback',
    modified: '2026-07-20T09:00:00Z',
    // One memory deliberately carries no section: the index links it above any
    // `##` heading, and the legend has to say so rather than inventing a bucket.
    section: i === 11 ? null : GRAPH_SECTIONS[i % GRAPH_SECTIONS.length],
    size: 1800 + i * 3200,
    in_degree: i === 0 ? 9 : 1,
    out_degree: i === 0 ? 3 : 1,
  })),
  // Usage is what the size selector reads. Varied on purpose: one memory
  // heavily used, one never touched, so "used" and "fresh" draw differently and
  // a control that silently did nothing would show up as an unchanged picture.
  usage: Object.fromEntries(
    Array.from({ length: 12 }, (_, i) => [
      `project_health_verified_core_lean_${i}`,
      {
        sessions: i === 0 ? 11 : i % 4,
        turns: i === 0 ? 180 : i * 3,
        reads: i % 3,
        edits: i === 0 ? 40 : i,
        last: i === 11 ? null : `2026-07-${String(10 + i).padStart(2, '0')}T09:00:00Z`,
      },
    ]),
  ),
  edges: [
    // Three dense groups of four…
    ...[0, 4, 8].flatMap((base) =>
      [
        [base, base + 1],
        [base + 1, base + 2],
        [base + 2, base + 3],
        [base + 3, base],
        [base, base + 2],
      ].map(([a, b]) => ({
        source: `project_health_verified_core_lean_${a}`,
        target: `project_health_verified_core_lean_${b}`,
      })),
    ),
    // …joined by two single links, which is what makes them separable at all.
    {
      source: 'project_health_verified_core_lean_0',
      target: 'project_health_verified_core_lean_4',
    },
    {
      source: 'project_health_verified_core_lean_4',
      target: 'project_health_verified_core_lean_8',
    },
  ],
};

/**
 * A timeline page whose every row is a wrapping risk at 390px: a minute, a
 * session name, a kind, a repository, a machine, a fold count and a verdict —
 * seven facts on one line. The session card in the console wrapped on exactly
 * four of those.
 */
const DOING = {
  moments: [
    {
      at: 29_412_600,
      agent: 'health',
      project: 'health',
      kind: 'test',
      n: 14,
      verdict: 'failed',
      effects: 936,
    },
    {
      at: 29_412_580,
      // A session that was never named shows its id, 36 characters of it.
      agent: '6f7c2f11-0000-4000-8000-000000000002',
      project: 'health-sync-backend',
      kind: 'build',
      n: 1,
      verdict: 'ok',
      // ⚠ The empty case, which is 12.6% of the live artefact and was the first
      // row Pippijn happened to tap. It must SAY so on the row.
      effects: 0,
    },
    {
      // Work on another machine, and a verdict that is neither ok nor failed.
      at: 29_412_540,
      agent: 'fleet',
      project: 'xinutec-infra',
      host: 'isis.xinutec.org',
      kind: 'deploy',
      n: 3,
      verdict: 'unknown',
      effects: 7,
    },
  ],
  summary: [
    ['test', 8021],
    ['build', 4310],
    ['edit', 299],
    ['deploy', 140],
  ],
  total: 15_570,
  failed: 412,
};

/** What one turn did — including the two things a summary would drop. */
const EFFECTS = {
  effects: [
    {
      at: 29_412_600,
      agent: 'health',
      // ⚠ The wire's letter, not the word. This fixture said `'wrote'` and the
      // test passed, because `models.ts` had been written from the same wrong
      // assumption — a fixture agreeing with the code is not evidence about the
      // wire. dev-lint's wire-mirror check is what caught it.
      did: 'w',
      path: 'health/packages/health-sync-backend/src/decode/quantiseLegCost.ts',
      command:
        "sed -i '' 's/Math.round/Math.floor/' packages/health-sync-backend/src/decode/quantiseLegCost.ts",
      reached: true,
      verdict: 'ok',
    },
    {
      at: 29_412_600,
      agent: 'health',
      did: 's',
      pattern: 'packages/**/*.spec.ts',
      // ⚠ A command after `&&` runs only if what preceded it worked.
      command: 'pnpm run verify && grep -rn quantiseLegCost packages/**/*.spec.ts',
      reached: false,
      verdict: 'unknown',
    },
    {
      // The subject nobody could name. Drawn, never dropped.
      at: 29_412_600,
      agent: 'health',
      did: 'u',
      command: 'nix develop --command bash -c "$(cat /tmp/step.sh)"',
      reached: true,
      verdict: 'ok',
    },
  ],
  total: 41,
  unnamed: 12,
};

/** Mock every backend call. Catch-all FIRST — Playwright runs handlers
 *  last-registered-first. */
async function mockApi(page: Page): Promise<void> {
  await page.route('**/api/**', (r) =>
    r.request().method() === 'GET' ? r.fulfill({ json: [] }) : r.fulfill({ status: 204, body: '' }),
  );
  await page.route('**/api/me', (r) => r.fulfill({ json: ME }));
  await page.route('**/api/index', (r) => r.fulfill({ json: INDEX }));
  await page.route('**/api/memories', (r) => r.fulfill({ json: MEMORIES }));
  await page.route('**/api/memory/**', (r) => r.fulfill({ json: MEMORY_PAGE }));
  await page.route('**/api/graph', (r) => r.fulfill({ json: GRAPH }));
  await page.route('**/api/search**', (r) => r.fulfill({ json: SEARCH }));
  await page.route('**/api/agents', (r) => r.fulfill({ json: AGENTS }));
  await page.route('**/api/doing**', (r) => r.fulfill({ json: DOING }));
  await page.route('**/api/effects**', (r) => r.fulfill({ json: EFFECTS }));
}

// The checker-checker: fail loudly here if the device preset is ever lost and
// the "phone width" suite silently runs at desktop width.
test('the suite really runs at phone geometry', async ({ page }) => {
  await mockApi(page);
  await page.goto('/');
  await expectViewportIsPhone(page);
});

test('index — dense link runs lay out cleanly @ phone width', async ({ page }, testInfo) => {
  await mockApi(page);
  await page.goto('/');
  await page.getByText('Memory index').waitFor();
  // The toolbar is where an icon-font fallback shows up as a literal word —
  // now "menu", since the destinations moved inside it and a closed mat-menu
  // renders nothing.
  await expectIconFontLoaded(page);
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo);
});

/**
 * Fenced code and wide tables in a memory body scroll horizontally BY DESIGN —
 * a shell command must not be reflowed to be readable. They're named explicitly
 * (the harness refuses to infer this from overflow-x, since overflow-y: auto
 * silently computes overflow-x to auto and would exempt half the page). The
 * containers themselves are still checked: only what scrolls INSIDE them is
 * allowed past the edge, so a `pre` that fails to clip still fails here.
 */
const MD_SCROLLERS = ['.md-content pre', '.md-content table'];

test('memory page — long slug, code, table, link panels @ phone width', async ({
  page,
}, testInfo) => {
  await mockApi(page);
  await page.goto('/m/project_health_verified_core_lean');
  await page.getByRole('heading', { name: 'project_health_verified_core_lean' }).waitFor();
  await page.getByText('Linked from').waitFor();
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo, null, MD_SCROLLERS);
});

test('all list — type filters + long slugs @ phone width', async ({ page }, testInfo) => {
  await mockApi(page);
  await page.goto('/all');
  await page.getByRole('button', { name: 'reference', exact: true }).waitFor();
  await page.getByText('user_cycling').waitFor();
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo);
});

/**
 * The agents page was the only one the harness never looked at, and it is the
 * densest text on the site: a totals line of five numbers and four phrases, then
 * a project row carrying a bar, two counts, a shell aside, an uncertainty aside
 * and a line delta. Every one of those is a wrapping risk at 390px.
 */
test('agents — a dense totals line and its provenance @ phone width', async ({
  page,
}, testInfo) => {
  await mockApi(page);
  await page.goto('/agents');
  // Scoped to the row, because the page's own lede explains the shell reader in
  // the same words the figure uses.
  await page.locator('.totals .via-shell').first().waitFor();
  await page.locator('.totals .maybe').first().waitFor();
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo);
});

/**
 * The timeline is the densest ROW on the site — a minute, a session, a kind, a
 * repository, a machine, a fold count and a verdict — and its evidence panel
 * carries verbatim shell commands, which are the longest unbreakable strings
 * this app renders anywhere.
 */
test('timeline — seven facts on a row, and a turn opened @ phone width', async ({
  page,
}, testInfo) => {
  await mockApi(page);
  await page.goto('/doing');

  // The shape of the WHOLE filtered range, which the rows cannot show.
  await page.getByText('15570 moments').waitFor();
  await page.locator('.moments > li').first().waitFor();
  // A session that was never named renders 36 characters of id.
  await page.getByText('6f7c2f11-0000-4000-8000-000000000002').waitFor();
  // ⚠ What opening a row would show, BEFORE it is opened — including the 0.
  // Both branches on screen at once, or the empty one is never rendered here.
  await page.getByText('936 effects').waitFor();
  await page.getByText('no evidence').waitFor();
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo);

  // ⚠ **Wait for the ANSWER, not for the row.** What this page draws while the
  // effects request is in flight is a progress bar, and asserting layout on it
  // would be measuring the pending state — the whole of memview#735 was two
  // checks that settled on their own race.
  await page.locator('.moments > li').first().locator('.row').click();
  await page.locator('.evidence .effects > li').first().waitFor();
  await page.getByText('quantiseLegCost.ts', { exact: false }).first().waitFor();

  // ⚠ The WORD, never the wire's letter. The artefact renames every variant to
  // one character because it is read over a VPN; a page drawing that straight
  // said `w` and `s` at the reader.
  await page.getByText('wrote', { exact: true }).waitFor();
  await page.getByText('searched', { exact: true }).waitFor();

  // The two things a summary would drop, and the reason this panel exists.
  await page.getByText('may not have run').waitFor();
  await page.getByText('and 12 more this could not name a subject for').waitFor();

  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo);
});

test('search results — snippets under long slugs @ phone width', async ({ page }, testInfo) => {
  await mockApi(page);
  await page.goto('/search?q=lean');
  await page.getByText('BigInt quant twin', { exact: false }).waitFor();
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo);
});

test('graph — cluster legend of long slugs under the canvas @ phone width', async ({
  page,
}, testInfo) => {
  await mockApi(page);
  await page.goto('/graph');
  // Every reading of the size control, because each one re-renders the canvas
  // and any of them can be the one that throws on a memory with no usage.
  for (const label of ['edited', 'fresh', 'linked', 'used']) {
    await page.getByRole('button', { name: label, exact: true }).click();
  }
  // The legend names each cluster after its most-connected member, so what has
  // to fit is a full memory slug, not a hand-written section title.
  await page.getByRole('heading', { name: 'clusters' }).waitFor();
  await page.locator('.legend button').first().waitFor();
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo);
});

/**
 * The trail scrolls sideways BY DESIGN. A walk of six memories is six
 * unbreakable snake_case slugs, which is wider than a phone at any font size,
 * and truncating the walk would throw away how the reader got where they are —
 * which is half of what a path through the corpus tells you.
 */
const TRAIL_SCROLLER = ['.trail ol'];

test('graph — a walk: trail crumbs and hop list @ phone width', async ({ page }, testInfo) => {
  await mockApi(page);
  // Entered by URL, then walked by clicking a hop. Clicking the canvas instead
  // would mean picking a pixel, and which pixel a node lands on depends on where
  // the force layout happened to settle — that measures the simulation.
  await page.goto('/graph?walk=project_health_verified_core_lean_0');
  await page.getByText('one hop away').waitFor();
  await page.locator('.hops button').first().click();
  // Two crumbs now: the walk was extended, not replaced.
  await page.locator('.trail li').nth(1).waitFor();
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo, null, TRAIL_SCROLLER);
});

test('graph — a linked walk survives a cold load @ phone width', async ({ page }, testInfo) => {
  await mockApi(page);
  await page.goto(
    '/graph?walk=project_health_verified_core_lean_0,project_health_verified_core_lean_3',
  );
  // Both crumbs, and the walk standing on the second one. The ordering this
  // pins is the whole risk: the URL is read before the corpus arrives, and a
  // walk checked against an empty graph drops every name it has and lands on
  // an unfocused picture with no sign the link ever said otherwise.
  await page.locator('.trail li').nth(1).waitFor();
  await page
    .getByRole('heading', { name: 'project_health_verified_core_lean_3', exact: true })
    .waitFor();
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo, null, TRAIL_SCROLLER);
});

/**
 * The graph is painted on a canvas, which is the one place the stylesheet does
 * not reach: an unparseable colour assigned to `fillStyle` is ignored in
 * SILENCE, leaving the previous value (black on a fresh context). Nothing else
 * in this suite can see that — the layout checks measure geometry, the unit
 * tests never rasterise, and the page is perfectly valid. So this reads pixels.
 *
 * Both schemes, because the classic form of the bug (a Material token, which
 * computes to `light-dark(...)`) is invisible in light mode.
 */
for (const scheme of ['light', 'dark'] as const) {
  test(`graph canvas stays legible in ${scheme} mode`, async ({ page }, testInfo) => {
    await page.emulateMedia({ colorScheme: scheme });
    await mockApi(page);
    await page.goto('/graph');
    await page.locator('app-graph-view canvas').waitFor();
    // The force layout settles over a few frames; measure once it has drawn.
    await page.waitForTimeout(1200);
    await expectCanvasLegible(page, testInfo, 'app-graph-view canvas');
  });
}

// ⚠ The failure states, which no other case reaches: every test above mocks a
// backend that answers. They are the states most likely to be wrong on a phone,
// because they are the ones nobody looks at — and a sentence plus a button is
// exactly the shape that wraps badly at 412px.
test('search — a failed search says so rather than "No matches." @ phone width', async ({
  page,
}, testInfo) => {
  await mockApi(page);
  await page.route('**/api/search**', (r) => r.fulfill({ status: 500, body: 'boom' }));
  await page.goto('/search?q=lean');
  await page.getByText("The search didn't run", { exact: false }).waitFor();
  // The claim this replaces must be absent: rendering both would be worse than
  // rendering only the wrong one.
  await expect(page.locator('.empty')).toHaveCount(0);
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo);
});

test('memory — a failed load is not "hasn\'t been written yet" @ phone width', async ({
  page,
}, testInfo) => {
  await mockApi(page);
  await page.route('**/api/memory/**', (r) => r.fulfill({ status: 500, body: 'boom' }));
  await page.goto('/m/project_health_verified_core_lean');
  await page.getByText("didn't load", { exact: false }).waitFor();
  await expect(page.getByText('marks something worth writing')).toHaveCount(0);
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo);
});

test('memory — a 404 still reads as not yet written @ phone width', async ({ page }, testInfo) => {
  await mockApi(page);
  await page.route('**/api/memory/**', (r) => r.fulfill({ status: 404, body: 'no such memory' }));
  await page.goto('/m/project_never_written');
  await page.getByText('marks something worth writing', { exact: false }).waitFor();
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo);
});
