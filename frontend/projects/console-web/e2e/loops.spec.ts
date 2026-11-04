import {
  NEXT,
  RUNNING,
  SHEET,
  expect,
  expectCleanLayout,
  expectThumbTargets,
  mockParse,
  mockRunner,
  routeStream,
  settleTransforms,
  test,
  type Parsed,
  type Timed,
} from './console';
import LOOP_GOLDEN from './parsed-loop.fixture.json';

test.use({ serviceWorkers: 'block' });

// dev-lint: allow-untyped-mock a JSON import widens every literal; console/tests/suite/parse.rs pins its shape
const LOOP: Parsed = LOOP_GOLDEN as Parsed;

/** The command `parsed-loop.fixture.json` is the runner's answer for. */
const COMMAND =
  'for r in coach health life memview messages tasks; do git -C ~/Code/$r status --short > /tmp/status-$r.txt; wc -l /tmp/status-$r.txt; done';

async function openLoop(page: import('@playwright/test').Page): Promise<void> {
  await mockRunner(page);
  await mockParse(page, LOOP);
  await routeStream(page, (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: (
        [
          { kind: 'started', model: 'claude-opus-5[1m]', cwd: '/home/example/Code', tools: 14 },
          {
            kind: 'tool',
            id: 'b1',
            name: 'Bash',
            does: { kind: 'bash', command: COMMAND },
            at: NEXT,
          },
        ] satisfies Timed[]
      )
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('.entry.tool button.opens').click();
  await page.locator(`${SHEET} .summary`).waitFor();
  await settleTransforms(page);
}

test('a loop run out is drawn as its body once, the value as a slot @ phone width', async ({
  page,
}, testInfo) => {
  await openLoop(page);
  const sheet = page.locator(SHEET);

  // The head, then the body once: thirteen steps drawn as three.
  await expect(sheet.locator('.step')).toHaveCount(3);
  await expect(sheet.locator('.step .kind').first()).toHaveText('loop');
  await expect(sheet.locator('.pass')).toHaveText(
    '6 passes · r = coach, health, life, memview, messages, tasks',
  );
  // The words exactly, slot included: a stray space inside a path is the failure.
  await expect(sheet.locator('.words').nth(1)).toHaveText('git -C ~/Code/r status --short');
  // Every pass's write is in sight, as one path with the slot in it.
  await expect(sheet.locator('.used.wrote .path')).toHaveText('/tmp/status-r.txt');
  await expect(sheet.locator('.used.wrote .path .slot')).toHaveText('r');
  await page.screenshot({ path: testInfo.outputPath('shape.png') });
  await expectThumbTargets(page);
  await expectCleanLayout(page, testInfo, { root: SHEET });
});
