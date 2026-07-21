import { test, type Page } from "@playwright/test";
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
