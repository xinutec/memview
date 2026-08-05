/**
 * Whether the newest message should be kept in view, and when to stop.
 *
 * A transcript that does not follow opens a hundred turns behind the present and
 * reads as a broken page; one that always follows yanks the view out from under
 * somebody reading back through the morning.
 *
 * ⚠ **The rule is the narrow one every other app of this kind uses**, and it is
 * worth stating precisely because a wider one was tried first: *when new content
 * arrives, if the view was already at the end, keep it at the end.* Messages, a
 * terminal, Slack — scroll up by a line in any of them and following stops, and
 * none of them ever scrolls you back.
 *
 * ⚠ **What the wider rule cost.** This used to re-decide, after every change,
 * whether the reader still counted as being at the end — and since the change
 * itself moves the end, the measurement was contaminated by the thing that
 * triggered it. Compensating for that took a gesture flag, two thresholds and
 * 300px of slack, and the result was a page that pulled you back down unless you
 * scrolled most of a screen: reported as "I need to scroll up quite a lot, then
 * it won't do that". None of that machinery survives here. The one piece kept
 * from it is [`wrote`], because the race it answers is real.
 *
 * ⚠ **Pure, and separated from the view for a reason.** Every rule was arrived
 * at from a measurement on a phone, and none of them could be tested where they
 * used to live: jsdom has no layout, so a component test cannot make a scroll
 * happen, and the layout harness hands a transcript over in one chunk where the
 * runner streams it. As a state machine fed positions, the same rules are
 * ordinary arithmetic — the numbers in the comments are what the tests replay.
 *
 * The view owns the box and does the reading and writing; this owns the
 * decision.
 */

/** What a scrolling box says about itself, in the three numbers that matter. */
export interface Box {
  /** `scrollTop`. */
  top: number;
  /** `scrollHeight` — everything there is, not what is on screen. */
  height: number;
  /** `clientHeight` — the window onto it. */
  view: number;
}

/** The three numbers, off a real element. Here rather than in the view because
 *  the shape and the reading of it belong together — a `Box` filled in wrongly
 *  is a decision made from the wrong measurement. */
export function measure(box: HTMLElement): Box {
  return { top: box.scrollTop, height: box.scrollHeight, view: box.clientHeight };
}

/**
 * How near the end still counts as being at it.
 *
 * ⚠ **Under a line, where this used to be 120px for arriving and 300px for
 * leaving.** Those were sized around the browser's own scroll anchoring, which
 * moved the position 18 or 19px unasked — and anchoring is now off for this list
 * (see `session-view.scss`, which says why that is safe here). What is left to
 * cover is rounding and a stray pixel, so scrolling up by one line stops the
 * page following, because that is what scrolling up by one line means
 * everywhere else on the phone.
 */
const SLACK = 16;

/** Whether a box is showing its own end. */
function atEnd(box: Box): boolean {
  return box.height - box.top - box.view < SLACK;
}

export class Following {
  /**
   * Whether the view was at the end when it was last looked at.
   *
   * ⚠ **Remembered as they scroll, not measured when it is wanted**, and the
   * distinction is the whole design. Growth does not fire a scroll event, so a
   * remembered answer survives the transcript getting longer underneath a reader
   * who has not moved, where a fresh measurement would watch the end run away
   * from them and call it leaving. It also survives the soft keyboard, which
   * takes half the screen: measured afresh at that moment, a reader who has not
   * moved is several hundred pixels from the bottom, and the message they tapped
   * the box to answer slides off the screen exactly as they start typing.
   */
  private at = true;

  /**
   * Whether a finger is on the transcript right now.
   *
   * ⚠ **Holding the screen is how a person stops a moving page, and it was the
   * one gesture that did nothing.** Reported from the phone: a session writing
   * its answer pulled the view to the end on every delta, including while the
   * reader had a thumb on the glass reading the sentence as it arrived. Nothing
   * else catches it — they have not scrolled, so they are still at the end, and
   * being at the end is precisely what makes the view move.
   *
   * So a hold *suspends* following rather than ending it: while the finger is
   * down nothing is written, and letting go resumes wherever the conversation
   * has got to. Suspending rather than stopping is what keeps a tap on a tool
   * row from meaning "leave me here" — a tap is a hold that lasts a moment, and
   * it ends with the view catching up as though nothing had happened.
   */
  private holding = false;

  /**
   * Whether the view moved under the finger that is on it.
   *
   * ⚠ **A hold and a drag are not the same gesture, and treating them alike put
   * the view back at the end the moment somebody let go of a scroll.** Catching
   * up on release is right for a hold — they stopped the page to read a line and
   * then let it go — and wrong for a drag, where letting go is simply the end of
   * the scroll they just performed.
   */
  private dragged = false;

  /**
   * The last position this engine asked for, or -1 for none outstanding.
   *
   * ⚠ **The one piece of the old machinery still needed.** The view is set to
   * the bottom and the browser queues a scroll event; more of the answer renders
   * before that event is delivered; the handler then runs against the NEW height
   * and the OLD position and reads one or two deltas' worth of gap — 120px to
   * 168px, measured — as a reader walking away. It bit two runs in five. The
   * position carried by that event is exactly where this engine put it, which is
   * what tells the two apart.
   *
   * Kept rather than cleared once used: the race can follow any write, not only
   * the first.
   */
  private wrote = -1;

  /** Whether the first render has happened; before it there is nothing to keep. */
  private started = false;

  /** Whether the reader is meant to be at the newest message. */
  get pinned(): boolean {
    return this.at;
  }

  /** Whether the view has been positioned at least once. */
  get settled(): boolean {
    return this.started;
  }

  /** Whether following is suspended by a finger on the screen. */
  get held(): boolean {
    return this.holding;
  }

  /**
   * Where the view should be put, or `undefined` to leave it where it is.
   *
   * The first positioning is not refused for anything: a transcript has to open
   * at its newest message, and a finger that happens to be down while it does is
   * not a decision about a page that is not on screen yet.
   */
  wants(box: Box): number | undefined {
    if (!this.started) return box.height;
    if (this.holding) return undefined;
    return this.at ? box.height : undefined;
  }

  /** Where the view actually landed, which is not always what was asked for —
   *  a box clamps `scrollTop` to what it can show. */
  landed(top: number): void {
    this.wrote = top;
    this.started = true;
  }

  /** A finger went down on the transcript. */
  took(): void {
    this.holding = true;
    this.dragged = false;
  }

  /**
   * And came off it.
   *
   * A hold that moved nothing leaves everything as it was, so a reader who was
   * following still is and the view catches up. A hold that *scrolled* is a
   * decision about where to be, and is answered from where they let go.
   */
  released(box: Box): void {
    this.holding = false;
    if (this.dragged) this.at = atEnd(box);
  }

  /**
   * The view moved.
   *
   * Whether that leaves the reader at the end is the whole question, and it is
   * asked of where they are rather than of how far they travelled to get there.
   */
  moved(box: Box): void {
    // ⚠ **Only while following**, because that is the only time anything is
    // written — see [`wrote`]. Once the reader has scrolled away nothing moves
    // the view but them, so a scroll that happens to land on the last position
    // this engine used is them coming back, and ignoring it would leave a reader
    // standing at the newest message with the page refusing to follow.
    if (this.at && box.top === this.wrote) return;
    // A drag: a finger is down and the position changed. See [`dragged`].
    if (this.holding) this.dragged = true;
    this.at = atEnd(box);
  }
}
