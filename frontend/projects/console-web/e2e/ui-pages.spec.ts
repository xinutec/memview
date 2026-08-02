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
 * Phone-width layout harness for the console. Render the real screens at a Pixel
 * viewport with the runner mocked and BUSY data, and assert the failure classes
 * that read fine in source and only show on a real phone.
 *
 * The at-risk strings here are not prose. A session is named by a filesystem
 * path with no spaces in it; a transcript is a column of tool arguments that are
 * paths, shell command lines and long flags. Those are the things that push a
 * page sideways, and this app puts them on a 412px screen on purpose.
 */
test.use({ serviceWorkers: "block" });

/** Material's indeterminate progress bar animates by translating its two bars
 *  right across the viewport and off the left edge — that is how the animation
 *  works, not a layout fault, and it is only on screen while a session is
 *  working. Allowed by selector rather than by turning the check off, so a real
 *  overflow on the same page still fails. */
const BUSY_BAR = ["mat-progress-bar"];

const REPOS = [
  "/home/example/Code/health",
  "/home/example/Code/memview",
  "/home/example/Code/nixos-config",
];

/** A roster with the shapes that crowd a narrow screen: a deep path, a long
 *  first instruction, a session that is working and one that has ended. */
const STATE = {
  dirs: ["/home/example/Code"],
  repos: REPOS,
  sessions: [
    {
      id: "6f7c2f11-0000-4000-8000-000000000001",
      dir: "/home/example/Code/health/packages/health-sync-backend/src/decode",
      started: 1785600000,
      alive: true,
      model: "claude-opus-5[1m]",
      busy: "requesting",
      turns: 12,
      cost_usd: 4.2137,
      waiting: 1,
      asked: "Port the remaining matcher gate to Lean and prove it bit-exact against the TypeScript quant twin, then run the golden set and report which journeys moved.",
    },
    {
      id: "6f7c2f11-0000-4000-8000-000000000002",
      dir: "/home/example/Code/memview",
      started: 1785599000,
      alive: false,
      model: "claude-haiku-4-5-20251001",
      turns: 3,
      cost_usd: 0.0084,
      waiting: 0,
      asked: "check the corpus",
    },
  ],
};

/** A transcript with every entry kind, and the arguments that are hardest to
 *  fit: an absolute path, a piped shell command, a failed call. */
const TRANSCRIPT = [
  { kind: "started", model: "claude-opus-5[1m]", cwd: STATE.sessions[0].dir, tools: 30 },
  {
    kind: "prompt",
    text: "Port the remaining matcher gate to Lean and prove it bit-exact against the TypeScript quant twin.",
  },
  { kind: "thinking", text: "The gate lives in decode/gate.ts and the Lean side already has the population step, so " },
  { kind: "thinking", text: "the missing piece is the leg quantisation cost." },
  {
    kind: "tool",
    id: "toolu_01",
    name: "Read",
    input: { file_path: "/home/example/Code/health/packages/health-sync-backend/src/decode/matcher-gate.ts" },
  },
  { kind: "tool_result", id: "toolu_01", ok: true },
  {
    kind: "tool",
    id: "toolu_02",
    name: "Bash",
    input: {
      command:
        "nix develop -c lake build && ./verified_cli match --serve --timeout 30000ms | tee /tmp/lean-gate.log",
    },
  },
  { kind: "tool_result", id: "toolu_02", ok: false },
  { kind: "text", text: "The build fails on `quantiseLegCost`: the Lean version rounds half-to-even and " },
  { kind: "text", text: "the TypeScript one rounds half-away-from-zero, so the two disagree on exactly the ties." },
  { kind: "turn", cost_usd: 0.3312, turns: 1, duration_ms: 48210 },
  // A question, undecided: the widest thing on the page, since it carries a
  // whole command AND two buttons on one 412px line.
  {
    kind: "ask",
    id: "8ed3af09-323c-404b-8368-9682dca75d26",
    tool: "Bash",
    title: "Claude wants to run nix develop -c lake build --verbose 2>&1 | tee /tmp/lean.log",
    input: { command: "nix develop -c lake build --verbose 2>&1 | tee /tmp/lean.log" },
  },
];

/** Mock every backend call. Catch-all FIRST — Playwright runs handlers
 *  last-registered-first. The event stream is served as a complete SSE body, so
 *  the transcript renders without a live runner. */
async function mockRunner(page: Page): Promise<void> {
  await page.route("**/api/**", (r) =>
    r.request().method() === "GET" ? r.fulfill({ json: [] }) : r.fulfill({ status: 204, body: "" }),
  );
  await page.route("**/api/state", (r) => r.fulfill({ json: STATE }));
  await page.route("**/api/sessions/*/events", (r) =>
    r.fulfill({
      contentType: "text/event-stream",
      body: TRANSCRIPT.map((event) => `data: ${JSON.stringify(event)}\n\n`).join(""),
    }),
  );
}

// The checker-checker: fail loudly here if the device preset is ever lost and
// the "phone width" suite silently runs at desktop width.
test("the suite really runs at phone geometry", async ({ page }) => {
  await mockRunner(page);
  await page.goto("/");
  await expectViewportIsPhone(page);
});

test("session list — deep paths and long instructions @ phone width", async ({ page }, testInfo) => {
  await mockRunner(page);
  await page.goto("/");
  await page.getByText("decode").first().waitFor();
  await expectIconFontLoaded(page);
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo);
});

test("transcript — tool arguments and a fixed composer @ phone width", async ({ page }, testInfo) => {
  await mockRunner(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
  // The failed shell call is the widest thing on the page; wait for it rather
  // than for the first paint, or the checks run against half a transcript.
  await page.getByText("verified_cli").first().waitFor();
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo, null, BUSY_BAR);
});

test("transcript — an undecided question with its two buttons @ phone width", async ({
  page,
}, testInfo) => {
  // The one screen that must work under a thumb on a train: a long command and
  // two controls, on a narrow screen, with nothing pushed off the edge.
  await mockRunner(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page.getByRole("button", { name: "allow" }).waitFor();
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo, null, BUSY_BAR);
});

test("session list — a blocked session says so first @ phone width", async ({ page }, testInfo) => {
  await mockRunner(page);
  await page.goto("/");
  await page.getByText("waiting for you").waitFor();
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo);
});

test("transcript — thinking unfolded is the longest the page gets @ phone width", async ({
  page,
}, testInfo) => {
  await mockRunner(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
  await page.getByRole("button", { name: /show thinking/ }).click();
  await page.getByText("quantisation").first().waitFor();
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo, null, BUSY_BAR);
});
