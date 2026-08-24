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
    following.took(box(0, 5000));

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

    following.took(box(end(5000), 5000));

    expect(
      following.wants(box(end(5000), 5400)),
      'the answer grew; the view stays',
    ).toBeUndefined();
    expect(following.wants(box(end(5000), 6000)), 'and keeps staying').toBeUndefined();
  });

  it('catches up when a hold that moved nothing ends', () => {
    // Suspended, not stopped. Otherwise every tap on a tool row would mean
    // "leave me here", and the transcript would stop following for the rest of
    // the conversation on the strength of somebody opening a result.
    const following = opened(5000);

    following.took(box(end(5000), 5000));
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

    following.took(box(end(10000), 10000));
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

    following.took(box(5000, 10000));
    following.moved(box(end(10000), 10000));
    following.released(box(end(10000), 10000));

    expect(following.pinned).toBe(true);
  });

  it('is not a way back for a reader who had scrolled away', () => {
    // Resting a thumb on the screen is not a request to be returned to the end.
    const following = opened(10000);
    following.moved(box(2000, 10000));

    following.took(box(2000, 10000));
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

describe('following · a thumb on the glass, measured 2026-08-10', () => {
  // ⚠ **Replays the phone measurement that settled #116**, made by holding the
  // transcript still — deliberately not scrolling — while a session wrote into
  // it. The trace: `gap=19 top=110310 was=110316 wrote=110328 held=true
  // height=110938 view=609`. Six pixels of movement, eighteen from the last
  // write, against `SLACK = 16`. Following stopped and never resumed; the gap
  // ran to 1,879px through releases as well as holds, which is what a live
  // session reading as a dead page actually is.
  //
  // Three theories were written to this ticket before the measurement and all
  // three were wrong, so these numbers are the fixtures rather than round ones.
  const VIEW = 609;
  /** Where the engine put the view, and how tall the transcript was then. */
  const WROTE = { top: 110328, height: 110938, view: VIEW };
  /** Where the thumb had dragged it, and what the session had written by then. */
  const DRIFTED = 110310;
  const GREW = 112817;

  function held(): Following {
    const following = new Following();
    following.landed(WROTE.top);
    following.took(WROTE);
    return following;
  }

  it('does not unpin while the finger is still down', () => {
    // The question a hold asks is answered when the finger lifts. Answering it
    // per event is what no threshold can survive: a drag arrives as thirty small
    // movements and its first one is a thumb.
    const following = held();

    following.moved({ ...WROTE, top: 110316 });
    following.moved({ ...WROTE, top: DRIFTED });

    expect(following.pinned, 'suspended, not decided').toBe(true);
  });

  it('catches up when a thumb that drifted eighteen pixels lets go', () => {
    const following = held();
    following.moved({ ...WROTE, top: DRIFTED });

    following.released({ top: DRIFTED, height: WROTE.height, view: VIEW });

    expect(following.pinned).toBe(true);
  });

  it('catches up though the session wrote all through the hold', () => {
    // ⚠ The end ran 1,879px away from a reader who never moved. Judged against
    // the transcript they let go of, every hold longer than a moment is a drag —
    // the contamination this file's header describes, on the one path that had
    // not been fixed for it.
    const following = held();
    following.moved({ ...WROTE, top: DRIFTED });

    following.released({ top: DRIFTED, height: GREW, view: VIEW });

    expect(following.pinned).toBe(true);
    expect(following.wants({ top: DRIFTED, height: GREW, view: VIEW })).toBe(GREW);
  });

  it('still stops following when the hold was a real scroll back', () => {
    // What #82 exists to protect, and what SLOP must not cost: reading back is
    // hundreds of pixels, an order of magnitude clear of a resting thumb.
    const following = held();

    following.released({ top: WROTE.top - 800, height: GREW, view: VIEW });

    expect(following.pinned).toBe(false);
  });

  it('picks them up again when the drag ends where the end was', () => {
    // Dragging back down during a long hold: the end they are returning to is
    // the one they left, not the one the session has written since.
    const following = new Following();
    following.landed(WROTE.top);
    following.moved({ ...WROTE, top: WROTE.top - 800 });
    expect(following.pinned, 'gone').toBe(false);

    following.took({ top: WROTE.top - 800, height: WROTE.height, view: VIEW });
    following.released({ top: WROTE.top + 1, height: GREW, view: VIEW });

    expect(following.pinned).toBe(true);
  });
});

describe('following · the composer takes the window, measured 2026-08-11', () => {
  // ⚠ **Replays the phone measurement in memview#731.** Typing a message grows
  // the composer, which takes height from the transcript. The trace:
  //
  //     gap=44 top=138573 was=138617 wrote=138617 held=false view=562 height=139179
  //
  // `was == wrote` — the reader was exactly where the engine had put them and had
  // not moved a pixel. `view` was 606 with an empty composer; three lines of text
  // leaves 562. The gap that opens is 44, which is EXACTLY what the window lost.
  //
  // All three numbers fall together by the same amount, which is what makes the
  // arithmetic come out at +44 rather than +88, so they are taken from the trace
  // rather than constructed.
  const AT_END = { top: 138617, height: 139223, view: 606 };
  const COMPOSER_GREW = { top: 138573, height: 139179, view: 562 };

  it('keeps following when the composer takes height from the transcript', () => {
    const following = new Following();
    following.landed(AT_END.top);
    following.moved(AT_END);
    expect(following.pinned, 'not following to begin with').toBe(true);

    following.moved(COMPOSER_GREW);

    expect(following.pinned).toBe(true);
    expect(following.wants(COMPOSER_GREW), 'following, but declining to follow').toBe(
      COMPOSER_GREW.height,
    );
  });

  it('does not unpin on the settling event after the reshape either', () => {
    // The reshape can arrive as more than one event. The second carries the same
    // window and the same position, so re-anchoring `wrote` is what makes it the
    // engine's own — otherwise the gap the composer opened is read as a reader
    // walking away one event later, and the fix would only move the defect.
    const following = new Following();
    following.landed(AT_END.top);
    following.moved(AT_END);
    following.moved(COMPOSER_GREW);

    following.moved(COMPOSER_GREW);

    expect(following.pinned).toBe(true);
  });

  it('still stops following if they scroll back after the composer grew', () => {
    // Forgiving the reshape must not forgive the next move. A reader who scrolls
    // with a taller composer on screen is scrolling.
    const following = new Following();
    following.landed(AT_END.top);
    following.moved(AT_END);
    following.moved(COMPOSER_GREW);

    following.moved({ ...COMPOSER_GREW, top: COMPOSER_GREW.top - 400 });

    expect(following.pinned).toBe(false);
  });

  it('is not a way back for a reader who had already scrolled away', () => {
    // The keyboard opening under somebody reading the morning is not a request to
    // be taken to the newest message.
    const following = new Following();
    following.landed(AT_END.top);
    following.moved({ ...AT_END, top: AT_END.top - 4000 });
    expect(following.pinned, 'gone').toBe(false);

    following.moved({ ...COMPOSER_GREW, top: AT_END.top - 4000 });

    expect(following.pinned).toBe(false);
  });
});

describe('following · saying something', () => {
  // ⚠ **Measured on the phone, 2026-08-11 (#731).** Sending re-lays the page out
  // as the composer collapses, and the browser moves the position while it does:
  //
  //     unpinned gap=92 top=145066 was=145157 wrote=145157 held=false view=534
  //
  // 110ms after the tap, no finger down, and the previous position exactly where
  // this engine had put it — 91px nobody asked for. No threshold separates that
  // from a reader's first scroll, because from outside they are the same event.
  // What separates them is that this one was expected.
  it('survives the relayout that sending causes', () => {
    const following = new Following();
    following.landed(145157);
    following.moved({ top: 145157, height: 145692, view: 534 });

    following.spoke();
    following.moved({ top: 145066, height: 145692, view: 534 });

    expect(following.pinned, 'detached by its own relayout').toBe(true);
  });

  it('does not take a reader who had scrolled away back to the end', () => {
    // ⚠ **Sending PROTECTS following, it does not restore it** — Pippijn's rule.
    // A message sent from halfway up the morning arrives at the end whether or
    // not it is watched, and being yanked there is what #82 exists to prevent.
    const following = new Following();
    following.landed(end(10000));
    following.moved(box(2000, 10000));
    expect(following.pinned, 'gone').toBe(false);

    following.spoke();
    following.moved(box(1980, 10000));

    expect(following.pinned).toBe(false);
    expect(following.wants(box(1980, 10600))).toBeUndefined();
  });

  it('is spent on the move it was expecting, not on the next real scroll', () => {
    const following = new Following();
    following.landed(end(10000));
    following.spoke();
    following.moved({ top: end(10000) - 20, height: 10000, view: 600 });
    expect(following.pinned, 'the relayout was forgiven').toBe(true);

    following.moved({ top: end(10000) - 400, height: 10000, view: 600 });

    expect(following.pinned, 'forgave a scroll it was not owed').toBe(false);
  });
});
