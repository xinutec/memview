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
    for (const control of document.querySelectorAll("button, a[href]")) {
      const box = control.getBoundingClientRect();
      // Not rendered at all — a disabled control in a collapsed branch.
      if (box.width === 0 && box.height === 0) continue;
      if (box.height >= least && box.width >= least) continue;
      missed.push({
        label: (control.textContent ?? "").trim().slice(0, 40) || control.className,
        width: Math.round(box.width),
        height: Math.round(box.height),
      });
    }
    return missed;
  }, min);
  expect(small, `controls smaller than ${min}px in either direction`).toEqual([]);
}

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
).join("\n");

/** A transcript with every entry kind, and the arguments that are hardest to
 *  fit: an absolute path, a piped shell command, a failed call, a cut tool
 *  result, and an answer carrying a fenced code block of unbreakable lines. */
const TRANSCRIPT = [
  { kind: "started", model: "claude-opus-5[1m]", cwd: STATE.sessions[0].dir, tools: 30, at: LATE },
  {
    kind: "prompt",
    text: "Port the remaining matcher gate to Lean and prove it bit-exact against the TypeScript quant twin.",
    at: LATE,
  },
  { kind: "thinking", text: "The gate lives in decode/gate.ts and the Lean side already has the population step, so ", at: LATE },
  { kind: "thinking", text: "the missing piece is the leg quantisation cost." },
  {
    kind: "tool",
    id: "toolu_01",
    name: "Read",
    input: { file_path: "/home/example/Code/health/packages/health-sync-backend/src/decode/matcher-gate.ts" },
    at: NEXT,
  },
  // A result the runner had to cut: unwrapped source, which is the widest single
  // thing this page can be asked to hold.
  { kind: "tool_result", id: "toolu_01", ok: true, detail: LONG_RESULT, cut: 48213 },
  {
    kind: "tool",
    id: "toolu_02",
    name: "Bash",
    input: {
      command:
        "nix develop -c lake build && ./verified_cli match --serve --timeout 30000ms | tee /tmp/lean-gate.log",
    },
  },
  // A failure, whose result the page opens by default — the one somebody
  // scrolled back to read.
  {
    kind: "tool_result",
    id: "toolu_02",
    ok: false,
    detail: "error: unknown flag --serve\nnote: run with --help for a list\nexit status 2",
  },
  { kind: "text", text: "The build fails on `quantiseLegCost`: the Lean version rounds half-to-even and " },
  { kind: "text", text: "the TypeScript one rounds half-away-from-zero, so the two disagree on exactly the ties." },
  // An answer carrying code. A fenced block is one unbreakable line, and it is
  // the commonest thing an answer holds that a 412px screen cannot fit.
  {
    kind: "text",
    text:
      "\n\n```ts\nconst quantiseLegCost = (leg: MatcherLeg, budget: number): number => Math.round(leg.seconds * budget * QUANT_SCALE) / QUANT_SCALE;\n```\n",
  },
  { kind: "turn", cost_usd: 0.3312, turns: 1, duration_ms: 48210, at: NEXT },
  // A question, undecided: the widest thing on the page, since it carries a
  // whole command AND two buttons on one 412px line.
  {
    kind: "ask",
    id: "8ed3af09-323c-404b-8368-9682dca75d26",
    tool: "Bash",
    title: "Claude wants to run nix develop -c lake build --verbose 2>&1 | tee /tmp/lean.log",
    input: { command: "nix develop -c lake build --verbose 2>&1 | tee /tmp/lean.log" },
    at: NEXT,
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
  await expectThumbTargets(page);
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
  // The decision and the send button on one screen: the controls whose size is
  // the difference between answering from a train and waiting until you are off
  // it.
  await expectThumbTargets(page);
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

test("the composer grows with what is being typed @ phone width", async ({ page }, testInfo) => {
  // The reason this exists: `rows="1"` meant a long instruction was written
  // through a one-line slit, on the device the console is driven from. jsdom has
  // no layout, so only a real browser can say whether the box actually grew.
  await mockRunner(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
  const box = page.locator("textarea");
  await box.waitFor();
  const oneLine = (await box.boundingBox())!.height;

  await box.fill("port the gate\nprove it bit-exact\nthen run the golden set\nand report what moved");
  const grown = (await box.boundingBox())!.height;
  expect(grown, "four lines have to be visible as four lines").toBeGreaterThan(oneLine);

  // And it stops. Past a third of the screen the composer is hiding the thing
  // being replied to, which is worse than not seeing the whole instruction.
  await box.fill(Array.from({ length: 40 }, (_, line) => `line ${line}`).join("\n"));
  const capped = (await box.boundingBox())!.height;
  const viewport = page.viewportSize()!.height;
  expect(capped, "the composer must not eat the transcript").toBeLessThan(viewport / 2);

  await expectNoHorizontalOverflow(page, testInfo, null, BUSY_BAR);
  await expectThumbTargets(page);
});

test("a tool result opens without widening the page @ phone width", async ({ page }, testInfo) => {
  // Unwrapped source, two thousand characters of it, on a 412px screen — the
  // widest single thing the transcript can be asked to hold.
  await mockRunner(page);
  await page.goto(`/s/${STATE.sessions[0].id}`);
  const unfold = page.getByRole("button", { name: /characters/ });
  await unfold.waitFor();
  await unfold.click();
  await page.getByText("quantiseLegCost", { exact: false }).first().waitFor();
  await expectNoTextOverlaps(page, testInfo);
  await expectNoHorizontalOverflow(page, testInfo, null, BUSY_BAR);
});
