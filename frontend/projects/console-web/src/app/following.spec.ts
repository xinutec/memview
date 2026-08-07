import { describe, expect, it } from 'vitest';

import { Box, Following } from './following';

/** A transcript of `height` in a `view`-tall window, read at `top`. */
function box(top: number, height: number, view = 600): Box {
  return { top, height, view };
}

/** The position that shows the end of a `height`-tall transcript. */
function end(height: number, view = 600): number {
  return height - view;
}

/** Open a transcript the way the view does: ask where to go, put it there,
 *  report where it landed. A box clamps, so what lands is the end. */
function opened(height: number, view = 600): Following {
  const following = new Following();
  const to = following.wants(box(0, height, view));
  expect(to, 'a transcript opens at its newest message').toBe(height);
  following.landed(end(height, view));
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

describe('scrolling away', () => {
  it('stops following as soon as they leave the end', () => {
    // ⚠ **A line, not a screen.** This wanted 300px of travel before it would
    // believe somebody had scrolled up, so a small scroll ended with the page
    // pulling itself back down — reported as "I need to scroll up quite a lot,
    // then it won't do that". Scrolling up by a line stops a terminal, a
    // messages app and a chat client following; there is no reason this should
    // ask for more.
    const following = opened(10000);

    following.moved(box(end(10000) - 40, 10000));

    expect(following.pinned).toBe(false);
    expect(following.wants(box(end(10000) - 40, 10600))).toBeUndefined();
  });

  it('follows again when they come back to the end', () => {
    const following = opened(10000);
    following.moved(box(5000, 10000));
    expect(following.pinned).toBe(false);

    following.moved(box(end(10000), 10000));

    expect(following.pinned).toBe(true);
  });

  it('counts a few pixels short of the end as the end', () => {
    // Sub-pixel rounding and a stray pixel of over-scroll, and nothing wider:
    // what the old slack was really covering was the browser's scroll
    // anchoring, and that is turned off for this list.
    const following = opened(10000);

    following.moved(box(end(10000) - 4, 10000));

    expect(following.pinned).toBe(true);
  });
});

describe('growth under a reader who has not moved', () => {
  it('does not count as leaving', () => {
    // ⚠ **The reason the answer is remembered rather than measured.** Growth
    // moves the end without firing a scroll event, so nothing here is asked
    // again — a fresh measurement would watch the end run away and call it
    // leaving. It was measured that way once: captures at 122, 125, 129, 130 and
    // 133 pixels from the end, none of them a person.
    const following = opened(5000);

    for (const height of [5122, 5400, 6000, 9000]) {
      expect(following.wants(box(end(5000), height)), `grew to ${height}`).toBe(height);
      following.landed(end(height));
    }
  });

  it('ignores the scroll event a write of its own causes', () => {
    // ⚠ The view is set to the bottom and the browser queues a scroll event;
    // more of the answer renders before that event is delivered; the handler
    // then runs against the NEW height and the OLD position, and one or two
    // deltas' worth of gap looks like a reader walking away. It bit two runs in
    // five.
    const following = opened(5000);

    following.moved(box(end(5000), 5168));

    expect(following.pinned, 'the position is exactly where it was put').toBe(true);
  });
});

describe('while the reader holds the screen', () => {
  it('stops writing scroll positions', () => {
    // ⚠ A session writing its answer pulled the view to the end on every delta,
    // including while the reader had a thumb on the glass reading the sentence
    // as it arrived — they had not scrolled, so they were still at the end, and
    // being at the end is exactly what moved the view.
    const following = opened(5000);

    following.took();

    expect(following.wants(box(end(5000), 5400)), 'the answer grew; the view stays').toBeUndefined();
    expect(following.wants(box(end(5000), 6000)), 'and keeps staying').toBeUndefined();
  });

  it('catches up when a hold that moved nothing ends', () => {
    // Suspended, not stopped. Otherwise every tap on a tool row would mean
    // "leave me here", and the transcript would stop following for the rest of
    // the conversation on the strength of somebody opening a result.
    const following = opened(5000);

    following.took();
    following.wants(box(end(5000), 6000));
    following.released(box(end(5000), 6000));

    expect(following.pinned).toBe(true);
    expect(following.wants(box(end(5000), 6000))).toBe(6000);
  });

  it('leaves a drag where it ended rather than snapping back', () => {
    // ⚠ Letting go of a drag is the end of a scroll, not the end of a pause.
    // The first version caught up on release whatever had happened during the
    // hold, which put the view straight back at the end.
    const following = opened(10000);

    following.took();
    following.moved(box(end(10000) - 200, 10000));
    following.released(box(end(10000) - 200, 10000));

    expect(following.pinned).toBe(false);
    expect(following.wants(box(end(10000) - 200, 10600))).toBeUndefined();
  });

  it('still catches up from a drag that ends at the end', () => {
    // Dragging back down to the newest message is how somebody says they have
    // finished reading back, and it is the one drag that should resume
    // following.
    const following = opened(10000);
    following.moved(box(5000, 10000));
    expect(following.pinned, 'gone').toBe(false);

    following.took();
    following.moved(box(end(10000), 10000));
    following.released(box(end(10000), 10000));

    expect(following.pinned).toBe(true);
  });

  it('is not a way back for a reader who had scrolled away', () => {
    // Resting a thumb on the screen is not a request to be returned to the end.
    const following = opened(10000);
    following.moved(box(2000, 10000));

    following.took();
    following.released(box(2000, 10000));

    expect(following.wants(box(2000, 10600))).toBeUndefined();
  });
});

describe('following · a gap the reader did not make', () => {
  // ⚠ **Replays the measurement that caused it**, from the console's own client
  // telemetry on 2026-08-07: `top=15730 height=16360 view=609`, a gap of 21 —
  // five pixels past SLACK — while `entries` climbed 163 → 164 → 165 and the
  // view never moved again. The transcript was live and read as a dead session.
  const AT_END = { top: 15730, height: 16360, view: 630 };

  it('keeps following when the viewport shrinks under a reader who has not moved', () => {
    const following = new Following();
    following.landed(AT_END.top);
    expect(following.pinned).toBe(true);

    // The URL bar slides in: `view` loses 21px, `top` does not move a pixel. On
    // a phone this fires a scroll event, and it is not a scroll.
    following.moved({ ...AT_END, view: 609 });
    expect(following.pinned, 'the box changed shape; the reader did not move').toBe(true);
  });

  it('keeps following when the conversation grows underneath', () => {
    const following = new Following();
    following.landed(AT_END.top);
    // Another entry arrives: the end runs away from a reader standing still.
    following.moved({ ...AT_END, height: AT_END.height + 240 });
    expect(following.pinned).toBe(true);
  });

  it('still stops following the moment they scroll back', () => {
    // The rule this must not break — one line up means leave me here, which is
    // what it means in every other app on the phone.
    const following = new Following();
    following.landed(AT_END.top);
    following.moved({ ...AT_END, top: AT_END.top - 40 });
    expect(following.pinned).toBe(false);
  });

  it('picks them up again when they scroll back down to the end', () => {
    // Once away, every move is theirs — including the one that comes back.
    const following = new Following();
    following.landed(AT_END.top);
    following.moved({ ...AT_END, top: AT_END.top - 400 });
    expect(following.pinned).toBe(false);
    following.moved(AT_END);
    expect(following.pinned).toBe(true);
  });
});
