import { describe, expect, it } from 'vitest';

import { Box, Following } from './following';

/** A transcript of `height` in a `view`-tall window, read at `top`. */
function box(top: number, height: number, view = 600): Box {
  return { top, height, view };
}

/** Open a transcript the way the view does: ask where to go, put it there,
 *  report where it landed. A box clamps, so what lands is the end. */
function opened(height: number, view = 600): Following {
  const following = new Following();
  const to = following.wants(box(0, height, view));
  expect(to, 'a transcript opens at its newest message').toBe(height);
  following.landed(height - view);
  return following;
}

describe('opening', () => {
  it('goes to the newest message rather than the top', () => {
    // A resumed conversation opens a hundred turns behind the present, and
    // scrolling to the end by hand every time reads as the page being broken.
    const following = new Following();

    expect(following.wants(box(0, 5000))).toBe(5000);
  });

  it('goes there even with a finger on the screen', () => {
    // A hold is a decision about a page you can see. Refusing the first
    // positioning because somebody happened to be touching the glass would open
    // the conversation at its oldest message instead.
    const following = new Following();
    following.took();

    expect(following.wants(box(0, 5000))).toBe(5000);
  });
});

describe('while the reader holds the screen', () => {
  it('stops writing scroll positions', () => {
    // ⚠ The reported defect. A session writing its answer pulled the view to the
    // end on every delta, including while the reader had a thumb on the glass
    // reading the sentence as it arrived — they had not scrolled, so they were
    // still pinned, and being pinned is exactly what moved the view.
    const following = opened(5000);

    following.took();

    expect(following.wants(box(4400, 5400)), 'the answer grew; the view stays').toBeUndefined();
    expect(following.wants(box(4400, 6000)), 'and keeps staying').toBeUndefined();
  });

  it('does not count as leaving, so letting go catches up', () => {
    // Suspended, not unpinned. Otherwise every tap on a tool row would mean
    // "leave me here", and the transcript would stop following for the rest of
    // the conversation on the strength of somebody opening a result.
    const following = opened(5000);

    following.took();
    following.wants(box(4400, 6000));
    following.released();

    expect(following.pinned).toBe(true);
    expect(following.wants(box(4400, 6000))).toBe(6000);
  });

  it('leaves a reader who scrolled away where they are', () => {
    // Holding is not a way back. Somebody who scrolled up to the morning and
    // then rested a thumb on the screen has not asked to be returned to the end.
    const following = opened(10000);
    following.byHand();
    following.moved(box(2000, 10000));
    expect(following.pinned, 'gone by 7400px').toBe(false);

    following.took();
    following.released();

    expect(following.wants(box(2000, 10600))).toBeUndefined();
  });
});

describe('what counts as the reader moving', () => {
  it('ignores a scroll with no gesture behind it', () => {
    // ⚠ Measured on the phone, three times in a row: the view was put at 1941
    // and the browser moved it to 1960 on its own — scroll anchoring, holding
    // the visible content still while the rest of the seed rendered. Read as a
    // reader's move, that unpinned a conversation 13% of the way down and left
    // it there.
    const following = opened(20000);

    following.moved(box(1960, 20000));

    expect(following.pinned).toBe(true);
  });

  it('ignores growth under a reader who never moved', () => {
    // ⚠ The other half of the same fault. While following, `scrollHeight` rises
    // before the scroll event is handled, so the distance from the end widens on
    // its own — captured at 122, 125, 129, 130 and 133 against a 120px slack,
    // none of them a person. Their `scrollTop` never changed, which is why
    // leaving is measured against where the view was put rather than against the
    // end.
    const following = opened(5000);
    following.byHand();

    for (const height of [5122, 5125, 5129, 5130, 5133]) {
      following.moved(box(4400, height));
      expect(following.pinned, `grew to ${height}`).toBe(true);
    }
  });

  it('stops following when they scroll back through the morning', () => {
    // Screens, not a fifth of one: the movement this is meant to catch is
    // somebody going to find something, and the view is 600px here.
    const following = opened(20000);
    following.byHand();

    following.moved(box(19400 - 1200, 20000));

    expect(following.pinned).toBe(false);
  });

  it('follows again when they come back to the end', () => {
    // Measured against the end rather than against where they left, because the
    // end is what they are coming back to and it has moved since.
    const following = opened(20000);
    following.byHand();
    following.moved(box(10000, 20000));
    expect(following.pinned).toBe(false);

    following.moved(box(25000 - 600 - 40, 25000));

    expect(following.pinned).toBe(true);
  });

  it('says nothing about a scroll it asked for itself', () => {
    // ⚠ The view is set to the bottom and the browser queues a scroll event;
    // more of the answer renders before that event is delivered; the handler
    // then runs against the NEW height and the OLD position and reads one or two
    // deltas' worth of gap as a reader walking away. It bit two runs in five.
    const following = opened(5000);
    following.byHand();

    following.moved(box(4400, 5168));

    expect(following.pinned, 'the position is exactly where it was put').toBe(true);
  });
});
