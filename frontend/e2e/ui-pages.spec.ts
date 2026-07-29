import { expect, test, type Page } from "@playwright/test";
// The fleet-shared harness, published as @xinutec/ui-harness (source repo
// ~/Code/ui-harness). Ships compiled JS, so it loads straight from node_modules.
import {
  expectIconFontLoaded,
  expectNoHorizontalOverflow,
  expectNoTextOverlaps,
  expectViewportIsPhone,
} from "@xinutec/ui-harness";

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
test.use({ serviceWorkers: "block" });

const ME = { user_id: "pippijn", display_name: "Pippijn", shared: false, auth_enabled: true };

/** The index as the backend renders it: dense interpunct-separated link runs
 *  with long slugs — the real MEMORY.md shape. */
const INDEX = {
  html: `<h1>Memory index</h1>
<p>Teasers only — OPEN the file before citing.</p>
<h2>Infrastructure &amp; data services</h2>
<ul><li><a href="/m/project_health_verified_core_lean">Lean verified core: matcher ported + Viterbi-argmax flagship wired into prod</a> · <a href="/m/reference_nixos_2605_dbus_broker_wedge">NixOS 26.05 dbus-broker wedge</a> · <a href="/m/project_fleet_firewall_hardening">firewall hardening (kubelet pending)</a> · <a href="/m/reference_launchd_tcc_external_volume">launchd + /Volumes/Backup: spawn exit 78</a></li></ul>`,
  count: 284,
};

const MEMORIES = [
  { name: "project_health_verified_core_lean", description: "Lean 4 port of the health matcher — bit-exact against the TypeScript quant twin", mtype: "project", modified: "2026-07-20T09:00:00Z" },
  { name: "reference_launchd_tcc_external_volume", description: "launchd jobs on /Volumes/Backup die with spawn exit 78; the child exec HANGS in dyld", mtype: "reference", modified: "2026-07-11T09:00:00Z" },
  { name: "feedback_no_magic_strings_use_upstream_taxonomy", description: "Don't hard-code a magic string from another service; use/extend its taxonomy at the source", mtype: "feedback", modified: "2026-07-02T09:00:00Z" },
  { name: "user_cycling", description: "\"cycling\" from the classifier is a misclassification", mtype: "user", modified: "2026-06-28T09:00:00Z" },
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
  dangling: ["project_lean_matcher_flip_soak"],
};

const SEARCH = {
  hits: [
    { ...MEMORIES[0], snippet: "…proved bit-exact against the BigInt quant twin; the flip gate records accepted deltas rather than silently breaking golden byte-identity…" },
    { ...MEMORIES[1], snippet: "…launchd + /Volumes/Backup: spawn exit 78; the child exec HANGS in dyld before main…" },
  ],
};

/** A graph the size the real one draws at: several sections (long titles, the
 *  legend's overflow risk), a hub, and a memory the index files under no
 *  heading. Enough nodes that the contrast probe below has pixels to measure. */
const GRAPH_SECTIONS = [
  "Infrastructure & data services",
  "Rules — deploy & infra ops",
  "health-sync",
];
const GRAPH = {
  sections: GRAPH_SECTIONS,
  nodes: Array.from({ length: 12 }, (_, i) => ({
    name: `project_health_verified_core_lean_${i}`,
    description: "Lean 4 port of the health matcher — bit-exact against the quant twin",
    mtype: i % 2 === 0 ? "project" : "feedback",
    modified: "2026-07-20T09:00:00Z",
    // One memory deliberately carries no section: the index links it above any
    // `##` heading, and the legend has to say so rather than inventing a bucket.
    section: i === 11 ? null : GRAPH_SECTIONS[i % GRAPH_SECTIONS.length],
    size: 1800 + i * 3200,
    in_degree: i === 0 ? 9 : 1,
    out_degree: i === 0 ? 3 : 1,
  })),
  edges: Array.from({ length: 11 }, (_, i) => ({
    source: "project_health_verified_core_lean_0",
    target: `project_health_verified_core_lean_${i + 1}`,
  })),
};

/** Mock every backend call. Catch-all FIRST — Playwright runs handlers
 *  last-registered-first. */
async function mockApi(page: Page): Promise<void> {
  await page.route("**/api/**", (r) =>
    r.request().method() === "GET" ? r.fulfill({ json: [] }) : r.fulfill({ status: 204, body: "" }),
  );
  await page.route("**/api/me", (r) => r.fulfill({ json: ME }));
  await page.route("**/api/index", (r) => r.fulfill({ json: INDEX }));
  await page.route("**/api/memories", (r) => r.fulfill({ json: MEMORIES }));
  await page.route("**/api/memory/**", (r) => r.fulfill({ json: MEMORY_PAGE }));
  await page.route("**/api/graph", (r) => r.fulfill({ json: GRAPH }));
  await page.route("**/api/search**", (r) => r.fulfill({ json: SEARCH }));
}

// The checker-checker: fail loudly here if the device preset is ever lost and
// the "phone width" suite silently runs at desktop width.
test("the suite really runs at phone geometry", async ({ page }) => {
  await mockApi(page);
  await page.goto("/");
  await expectViewportIsPhone(page);
});

test("index — dense link runs lay out cleanly @ phone width", async ({ page }, testInfo) => {
  await mockApi(page);
  await page.goto("/");
  await page.getByText("Memory index").waitFor();
  // The toolbar is where an icon-font fallback shows up as the literal words
  // "search"/"list"/"share"/"logout".
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
const MD_SCROLLERS = [".md-content pre", ".md-content table"];

test("memory page — long slug, code, table, link panels @ phone width", async ({ page }, testInfo) => {
  await mockApi(page);
  await page.goto("/m/project_health_verified_core_lean");
  await page.getByRole("heading", { name: "project_health_verified_core_lean" }).waitFor();
  await page.getByText("Linked from").waitFor();
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo, null, MD_SCROLLERS);
});

test("all list — type filters + long slugs @ phone width", async ({ page }, testInfo) => {
  await mockApi(page);
  await page.goto("/all");
  await page.getByRole("button", { name: "reference", exact: true }).waitFor();
  await page.getByText("user_cycling").waitFor();
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo);
});

test("search results — snippets under long slugs @ phone width", async ({ page }, testInfo) => {
  await mockApi(page);
  await page.goto("/search?q=lean");
  await page.getByText("BigInt quant twin", { exact: false }).waitFor();
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo);
});

test("graph — legend of long section titles under the canvas @ phone width", async ({ page }, testInfo) => {
  await mockApi(page);
  await page.goto("/graph");
  await page.getByText("Infrastructure & data services").waitFor();
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo);
});

/**
 * Canvas drawing takes colour strings, and an unparseable one is ignored in
 * silence — `fillStyle` simply keeps its previous value, which starts out black.
 * Material's system tokens compute to `light-dark(#1a1b1f, #e3e2e6)`, a CSS
 * function no canvas can parse, so passing one straight through paints black on
 * a dark background with nothing anywhere reporting a problem.
 *
 * Nothing else in this suite can see that: the layout checks measure geometry,
 * the unit tests never rasterise, and the page is perfectly valid. So this reads
 * the pixels, in both schemes — the bug is invisible in light mode.
 */
for (const scheme of ["light", "dark"] as const) {
  test(`graph canvas stays legible in ${scheme} mode`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: scheme });
    await mockApi(page);
    await page.goto("/graph");
    const canvas = page.locator("app-graph-view canvas");
    await canvas.waitFor();
    // The layout settles over a few frames; measure once it has drawn.
    await page.waitForTimeout(1200);

    const contrast = await canvas.evaluate((element) => {
      if (!(element instanceof HTMLCanvasElement)) throw new Error("not a canvas");
      const ctx = element.getContext("2d");
      if (!ctx) throw new Error("no 2d context");

      const relativeLuminance = (r: number, g: number, b: number): number => {
        const channel = (v: number): number => {
          const s = v / 255;
          return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
        };
        return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
      };

      const background = getComputedStyle(document.body).backgroundColor;
      const channels = background.match(/\d+/g)?.map(Number) ?? [255, 255, 255];
      const backgroundLuminance = relativeLuminance(channels[0], channels[1], channels[2]);

      const { data } = ctx.getImageData(0, 0, element.width, element.height);
      // Solidly painted pixels only — antialiased edges and the faint edge lines
      // blend toward the background by design and would drag the measure down.
      const ratios: number[] = [];
      for (let i = 0; i < data.length; i += 4) {
        if (data[i + 3] < 200) continue;
        const l = relativeLuminance(data[i], data[i + 1], data[i + 2]);
        const [hi, lo] =
          l > backgroundLuminance ? [l, backgroundLuminance] : [backgroundLuminance, l];
        ratios.push((hi + 0.05) / (lo + 0.05));
      }
      if (ratios.length === 0) return { painted: 0, best: 0 };
      ratios.sort((a, b) => a - b);
      return { painted: ratios.length, best: ratios[Math.floor(ratios.length * 0.9)] };
    });

    expect(contrast.painted, "the graph canvas painted nothing at all").toBeGreaterThan(200);
    expect(
      contrast.best,
      `graph canvas in ${scheme} mode: brightest marks reach only ${contrast.best.toFixed(1)}:1`,
    ).toBeGreaterThan(3);
  });
}
