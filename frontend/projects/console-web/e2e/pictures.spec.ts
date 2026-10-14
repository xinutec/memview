import {
  BUSY_BAR,
  NEXT,
  RUNNING,
  expect,
  expectCleanLayout,
  expectNoHorizontalOverflow,
  expectNoPinnedOverlap,
  expectThumbTargets,
  mockRunner,
  opened,
  placed,
  routeStream,
  settleTransforms,
  test,
  tinyPng,
  type Timed,
} from './console';

test.use({ serviceWorkers: 'block' });

test('a picture waits to be sent with what is said about it @ phone width', async ({
  page,
}, testInfo) => {
  // The phone is where the screen being talked about is. A layout that
  // settles wrongly, a chart that reads oddly, a thing on a desk — all of it was
  // describable and not showable until this. The picture is scaled in the page
  // (see `picture.ts`) and sent as an `image` block on the session's stdin, which
  // the CLI forwards and the model reads.
  let sent: Record<string, unknown> | undefined;
  await mockRunner(page);
  await page.route('**/api/sessions/*/image', (r) => {
    sent = r.request().postDataJSON() as Record<string, unknown>;
    return r.fulfill({ json: RUNNING });
  });
  await page.goto(`/s/${RUNNING.id}`);
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

  // The composer holds a thumbnail, a size, a discard button, the box and
  // send — the fullest this row ever gets, and the phone is 412px wide.
  await expectCleanLayout(page, testInfo, { allow: BUSY_BAR });
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

test('a picture that was sent is on the screen, not a path to it @ phone width', async ({
  page,
}, testInfo) => {
  // The half of the feature that was missing. A sent picture reached the
  // model and left the person who took it with a sentence about a file path —
  // the one party to the conversation who could not see it. The runner reads the
  // image block back out of the transcript, keeps the note out of the words, and
  // says which file; the page fetches it.
  let asked = '';
  await mockRunner(page);
  await routeStream(page, (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: (
        [
          { kind: 'started', model: 'claude-opus-5[1m]', cwd: '/home/example/Code', tools: 14 },
          { kind: 'shown', name: '2026-08-05-184700Z.png', at: NEXT },
          { kind: 'prompt', text: 'what is wrong with this?', at: NEXT },
        ] satisfies Timed[]
      )
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.route('**/api/sessions/*/images/*', (r) => {
    asked = r.request().url();
    return r.fulfill({ path: tinyPng(), contentType: 'image/png' });
  });
  await page.goto(`/s/${RUNNING.id}`);

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

  await expectCleanLayout(page, testInfo, { allow: BUSY_BAR });
});

test('a picture can be put down again without being sent @ phone width', async ({ page }) => {
  // The discard is not decoration: the picker is a gallery on a phone and the
  // wrong screenshot is one tap away from the right one.
  await mockRunner(page);
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('.composer').waitFor();
  await page.locator('.picker').setInputFiles(tinyPng());
  await page.locator('.chosen').waitFor();

  await page.getByRole('button', { name: 'do not send this image' }).click();

  await expect(page.locator('.chosen')).toHaveCount(0);
  // Send goes back to needing words, which is what it needed before there was a
  // picture to send on its own.
  await expect(page.locator('.composer .send')).toBeDisabled();
});

test('a link to a render opens over the conversation, and back puts it away @ phone width', async ({
  page,
}, testInfo) => {
  // The link was never dead — it went to the wrong place. GFM autolinks a
  // bare URL, and the shell hands any host it does not know to the phone's
  // browser: the addresses a session writes name the Mac's LAN, which the phone
  // is not on. So the tap left the console and arrived nowhere. What is checked
  // here is that it now goes through the console instead, and that the back
  // gesture puts the picture away without also leaving the conversation.
  const RENDER = 'http://10.0.0.2:8917/data/peek/peekA-350-view_from_sofa.png';
  let asked = '';
  await mockRunner(page);
  await routeStream(page, (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: (
        [
          { kind: 'started', model: 'claude-opus-5[1m]', cwd: '/home/example/Code', tools: 14 },
          // `text` and not `said`: the runner streams a model's words as deltas
          // and the page assembles them, which is the shape a real transcript has.
          { kind: 'text', text: `Rendered from the sofa: ${RENDER}`, at: NEXT },
        ] satisfies Timed[]
      )
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.route('**/api/picture*', (r) => {
    asked = r.request().url();
    return r.fulfill({ path: tinyPng(), contentType: 'image/png' });
  });
  await page.goto(`/s/${RUNNING.id}`);

  // The address is still what the sentence says — it is how a person tells two
  // renders of the same room apart.
  const link = page.locator('a.picture-link');
  await expect(link).toHaveText(RENDER);
  // And it points at the console, which is on that LAN. Read off the DOM rather
  // than trusted from the source: the sanitiser is between the two.
  expect(await link.getAttribute('href')).toContain('/api/picture?url=');

  // By keyboard, which is also the claim `session-view.ts` makes to justify
  // handling this on the host. The a11y rules that fire on a `(click)` in the
  // template are asking whether this can be reached without a pointer: focusing
  // the anchor and pressing Enter dispatches the same event the tap does, and
  // that is what the handler is waiting for.
  await link.focus();
  await page.keyboard.press('Enter');

  const shown = page.locator('ui-picture-sheet img');
  await shown.waitFor();
  // The sheet arrives by translating up the screen, and a box read mid-flight is
  // not the box the layout claims — see [[settleTransforms]].
  await settleTransforms(page);
  expect(asked, 'the console is asked for it, not the LAN host').toContain(
    encodeURIComponent(RENDER),
  );
  // Decoded, not merely requested: a picture that failed to load is a visible
  // element with no pixels in it, and every assertion about visibility passes on
  // one.
  expect(await shown.evaluate((img: HTMLImageElement) => img.naturalWidth)).toBe(2);

  // Back closes the sheet and stays in the conversation, though a sheet takes
  // no part in history. See `Sheets` in `@xinutec/ui-scaffold`.
  await page.goBack();
  await expect(page.locator('ui-picture-sheet')).toHaveCount(0);
  await expect(page).toHaveURL(new RegExp(`/s/${RUNNING.id}$`));
  await expect(page.locator('.entry.said')).toContainText('Rendered from the sofa');

  await expectCleanLayout(page, testInfo, { allow: BUSY_BAR });
});

test('a picture a session read is drawn small, and a tap opens it whole @ phone width', async ({
  page,
}, testInfo) => {
  const SHOT = '/tmp/usage-shape/red-dark.png';
  const asked: string[] = [];
  await mockRunner(page);
  await routeStream(page, (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: (
        [
          { kind: 'started', model: 'claude-opus-5[1m]', cwd: '/home/example/Code', tools: 14 },
          { kind: 'tool', id: 'r1', name: 'Read', does: { kind: 'other', shown: SHOT }, at: NEXT },
          { kind: 'tool_result', id: 'r1', ok: true, detail: '[an image]', image: true, at: NEXT },
        ] satisfies Timed[]
      )
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.route('**/api/picture*', (r) => {
    asked.push(r.request().url());
    return r.fulfill({ path: tinyPng(), contentType: 'image/png' });
  });
  await page.goto(`/s/${RUNNING.id}`);

  const thumb = page.locator('.entry.tool a.picture-link img');
  await thumb.waitFor();
  await expect.poll(() => asked.length, 'the thumbnail was not fetched').toBeGreaterThan(0);
  expect(asked[0], 'the small version, not the original').toContain('small=true');

  await thumb.click();
  const shown = page.locator('ui-picture-sheet img');
  await shown.waitFor();
  await settleTransforms(page);
  expect(asked.at(-1), 'the sheet asks for the original').not.toContain('small=true');
  expect(await shown.evaluate((img: HTMLImageElement) => img.naturalWidth)).toBe(2);

  await expectCleanLayout(page, testInfo, { root: 'ui-picture-sheet' });
  await expectNoHorizontalOverflow(page, testInfo, null, BUSY_BAR);
});

test("a render whose server is gone says so, in the console's words @ phone width", async ({
  page,
}, testInfo) => {
  // The ordinary failure, not the exotic one. These links outlive the
  // servers that answer them: a session renders, serves the file from a shell it
  // is still holding, and stops. What the console says about that has to tell
  // "re-render it" apart from "start the server again", and an `<img>` that
  // failed says neither — its error event carries nothing at all.
  await mockRunner(page);
  await routeStream(page, (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: (
        [
          { kind: 'started', model: 'claude-opus-5[1m]', cwd: '/home/example/Code', tools: 14 },
          { kind: 'text', text: 'http://10.0.0.2:8917/data/peek/gone.png', at: NEXT },
        ] satisfies Timed[]
      )
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.route('**/api/picture*', (r) =>
    r.fulfill({
      status: 502,
      contentType: 'text/plain',
      body: 'could not reach it: connection refused',
    }),
  );
  await page.goto(`/s/${RUNNING.id}`);
  await page.locator('a.picture-link').click();

  const sheet = page.locator('ui-picture-sheet');
  // The console's sentence, not the status: "the runner answered 502" names
  // neither of the two things it means.
  await expect(sheet).toContainText('could not reach it: connection refused');
  await expect(sheet).not.toContainText('502');

  await expectCleanLayout(page, testInfo, { root: 'ui-picture-sheet' });
  await expectNoHorizontalOverflow(page, testInfo, null, BUSY_BAR);
});

test('a picture the session named by its place on the disk opens too @ phone width', async ({
  page,
}, testInfo) => {
  // The shape a session writes when it is not also running a server, which
  // is most of the time: it has the file it just rendered. observe wrote
  // `![Photo: cabinet corner](/Users/…/lroom-at20s-photo-upright.jpg)` and the
  // tap did nothing useful — a path resolves against the console's own origin,
  // where it falls through to this app and reloads the page it was made from.
  const FILE = '/home/example/Code/observe/data/peek/lroom-at20s-photo-upright.jpg';
  let asked = '';
  await mockRunner(page);
  await routeStream(page, (r) =>
    r.fulfill({
      contentType: 'text/event-stream',
      body: (
        [
          { kind: 'started', model: 'claude-opus-5[1m]', cwd: '/home/example/Code', tools: 14 },
          { kind: 'text', text: `![Photo: cabinet corner](${FILE})`, at: NEXT },
        ] satisfies Timed[]
      )
        .map((event) => `data: ${JSON.stringify(event)}\n\n`)
        .join(''),
    }),
  );
  await page.route('**/api/picture*', (r) => {
    asked = r.request().url();
    return r.fulfill({ path: tinyPng(), contentType: 'image/png' });
  });
  await page.goto(`/s/${RUNNING.id}`);

  // The alt text is the label: it is what the author wrote it to be, and it
  // reads better than a path that fills three lines of a phone.
  await expect(page.locator('a.picture-link')).toHaveText('Photo: cabinet corner');
  await page.locator('a.picture-link').click();

  const shown = page.locator('ui-picture-sheet img');
  await shown.waitFor();
  expect(asked, 'the console is asked for the file, by its place on the disk').toContain(
    encodeURIComponent(FILE),
  );
  expect(await shown.evaluate((img: HTMLImageElement) => img.naturalWidth)).toBe(2);
  // The address bar above the picture says which file, since the link did not.
  await expect(page.locator('ui-picture-sheet .where')).toContainText('lroom-at20s');

  await expectCleanLayout(page, testInfo, { root: 'ui-picture-sheet' });
  await expectNoHorizontalOverflow(page, testInfo, null, BUSY_BAR);
});

test('a picture can be pinched closer and dragged about @ phone width', async ({
  page,
}, testInfo) => {
  // Real touches, through CDP, and not `dispatchEvent`. A synthesised
  // pointer has an id the browser does not know, so `setPointerCapture` throws
  // `NotFoundError` and the handler is abandoned before it records the finger —
  // the first cut of this test read scale 1 and looked like a broken pinch. What
  // `Input.dispatchTouchEvent` sends goes through the same path a thumb does,
  // capture included. `zoom.spec.ts` has the arithmetic; this has the wiring.
  await opened(page);
  expect(await placed(page), 'opens on the whole picture').toMatchObject({ scale: 1 });

  const touch = await page.context().newCDPSession(page);
  const box = await page.locator('.frame').boundingBox();
  const mid = {
    x: (box?.x ?? 0) + (box?.width ?? 0) / 2,
    y: (box?.y ?? 0) + (box?.height ?? 0) / 2,
  };
  const fingers = (spread: number) => [
    { x: mid.x - spread, y: mid.y, id: 1 },
    { x: mid.x + spread, y: mid.y, id: 2 },
  ];

  // Two fingers 60px apart, spread to 240: four times, about their middle.
  await touch.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: fingers(30) });
  await touch.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: fingers(120) });

  const close = await placed(page);
  expect(close.scale).toBeGreaterThan(3.9);
  expect(close.scale).toBeLessThan(4.1);

  await touch.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });

  // One finger after two. The pair has to be forgotten when a finger
  // lifts, or the next drag is read as a pinch against a finger that is gone.
  await touch.send('Input.dispatchTouchEvent', {
    type: 'touchStart',
    touchPoints: [{ x: mid.x, y: mid.y, id: 3 }],
  });
  await touch.send('Input.dispatchTouchEvent', {
    type: 'touchMove',
    touchPoints: [{ x: mid.x - 80, y: mid.y, id: 3 }],
  });
  await touch.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });

  const dragged = await placed(page);
  expect(dragged.x, 'the drag moved it left').toBeLessThan(close.x);
  expect(dragged.scale, 'and did not change how close it is').toBeCloseTo(close.scale, 3);

  // The drag must not also toggle. A pan ends with a `click` on the
  // element it started on, and without the slip guard the picture jumps back to
  // fitted every time it is moved — the gesture undoing itself as it finishes.
  expect(await placed(page)).toMatchObject({ scale: dragged.scale, x: dragged.x });

  await expectCleanLayout(page, testInfo, { root: 'ui-picture-sheet' });
  await expectNoHorizontalOverflow(page, testInfo, null, BUSY_BAR);
});

test('a tap looks closer, and a second tap shows the whole picture @ phone width', async ({
  page,
}) => {
  // The keyboard path is the same handler: the button takes focus and Enter
  // reaches it, which is what keeps a gesture-driven control usable without one.
  await opened(page);

  await page.locator('.frame').click();
  // Polled, not read once. The transform is written on the next render,
  // and an assertion in the same tick as the event reads the previous frame —
  // which is how a working toggle looked broken here for twenty minutes.
  await expect.poll(async () => (await placed(page)).scale).toBeGreaterThan(1);

  // The keyboard reaches the same handler: the picture is a button, so it takes
  // focus and Enter activates it. That is what keeps a control driven by
  // gestures usable without them.
  await page.locator('.frame').press('Enter');
  await expect.poll(() => placed(page)).toMatchObject({ scale: 1, x: 0, y: 0 });
});
