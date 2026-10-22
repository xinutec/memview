import { screens } from '@xinutec/ui-harness/routes';

import ROUTES from './routes.json';
import {
  NEXT,
  RUNNING,
  expect,
  expectCleanLayout,
  mockRunner,
  test,
  type Run,
  type Stretched,
} from './console';

test.use({ serviceWorkers: 'block' });

/** One workflow run with an agent in it, so its two screens have something to draw. */
const RUN = 'wf-sample';
const AGENT = 'a1';

/** Every screen the console routes to, from its route table (`routes.json`). */
for (const { route, url } of screens(ROUTES.routes, { id: RUNNING.id, run: RUN, agent: AGENT })) {
  test(`${route} lays out cleanly @ phone width`, async ({ page }, testInfo) => {
    await mockRunner(page);
    await page.route(`**/api/sessions/*/workflows/${RUN}`, (r) =>
      r.fulfill({
        json: {
          phases: [
            { title: 'Rewrite', agents: [{ id: AGENT, label: 'comments:core', done: true }] },
          ],
        } satisfies Run,
      }),
    );
    await page.route(`**/api/sessions/*/workflows/${RUN}/agents/${AGENT}*`, (r) =>
      r.fulfill({
        // A read past the end finds nothing new, as the runner's does.
        json: new URL(r.request().url()).searchParams.has('after')
          ? ({ events: [], from: 100, to: 100 } satisfies Stretched)
          : ({
              events: [{ kind: 'text', text: 'Reading the runner first.', at: NEXT }],
              from: 0,
              to: 100,
            } satisfies Stretched),
      }),
    );
    await page.goto(url);
    await expect(page.locator('ui-scaffold h1')).not.toBeEmpty();
    await expectCleanLayout(page, testInfo);
  });
}
