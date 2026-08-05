/**
 * Whether the newest message should be kept in view, and when to stop.
 *
 * A transcript that does not follow opens a hundred turns behind the present and
 * reads as a broken page; one that always follows yanks the view out from under
 * somebody reading back through the morning. Everything here is the second half
 * of that — the rules for when following is *wrong*.
 *
 * ⚠ **Pure, and separated from the view for a reason.** Every rule below was
 * arrived at from a measurement on a phone, and none of them could be tested
 * where they used to live: jsdom has no layout, so a component test cannot make
 * a scroll happen, and the layout harness hands a transcript over in one chunk
 * where the runner streams it. As a state machine fed positions, the same rules
 * are ordinary arithmetic — the numbers in the comments are what the tests
 * replay.
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

/** How near the end still counts as being at it: a few lines, so a
 *  partly-scrolled view is following rather than left behind. */
const NEAR = 120;

/**
 * And how far from where we put them counts as having left.
 *
 * ⚠ **The two directions do not share a threshold.** [`NEAR`] is the slack for
 * arriving at the end. Leaving it needs a bigger number: measured on the device,
 * the browser's own adjustments reach 122px, so a 120px slack calls a motionless
 * reader "gone" on the strength of movement they did not make. Somebody
 * genuinely scrolling back through the morning travels screens, not a fifth of
 * one — 609px is the viewport there — so this costs nothing real and puts the
 * decision well clear of the noise.
 */
const AWAY = 300;

export class Following {
  /**
   * Whether the reader is at the newest message.
   *
   * ⚠ **Remembered as they scroll, not measured when it is wanted.** It was
   * measured, and the moment it is wanted includes the soft keyboard opening: by
   * then the transcript has already lost half its height, so the arithmetic says
   * "several hundred pixels from the bottom" about a reader who has not moved —
   * and the message they tapped the box to answer slides off the screen exactly
   * as they start typing.
   */
  private at = true;

  /**
   * Whether the reader has ever scrolled this transcript by hand.
   *
   * ⚠ **Until they have, no scroll may unpin, whatever the position says.**
   * Measured on the phone, three times in a row: the view was scrolled to the
   * bottom of what had arrived and 1941 written down; the browser then moved it
   * to 1960 on its own — scroll anchoring, holding the visible content still
   * while the rest of the seed rendered around it. Eighteen or nineteen pixels,
   * every time. That made `top !== wrote`, which was the only test for "the
   * reader did this", so a 12,699px gap was measured against a 120px threshold,
   * following stopped, and the remaining 13,000 pixels arrived unwatched. The
   * conversation opened 13% of the way down and stayed there. Opening it again
   * was fine, because a cached transcript renders in one pass and there is no
   * growth for the browser to react to.
   *
   * A gesture is the thing layout cannot fake: a wheel, a drag, a key.
   */
  private gestured = false;

  /**
   * Whether a finger is on the transcript right now.
   *
   * ⚠ **Holding the screen is how a person stops a moving page, and it was the
   * one gesture that did nothing.** Reported from the phone: a session writing
   * its answer pulled the view to the end on every delta, including while the
   * reader had a thumb on the glass reading the sentence as it arrived. Nothing
   * above catches it — they have not scrolled, so they are still pinned, and
   * being pinned is precisely what makes the view move.
   *
   * So a hold *suspends* following rather than ending it: while the finger is
   * down nothing is written, and letting go resumes wherever the conversation
   * has got to. Suspending rather than unpinning is what keeps a tap on a tool
   * row from meaning "leave me here" — a tap is a hold that lasts a moment, and
   * it ends with the view catching up as though nothing had happened.
   */
  private holding = false;

  /**
   * The last position this engine asked for, or -1 for none outstanding.
   *
   * ⚠ **Kept, not cleared, once a scroll has been accounted for.** Clearing it
   * threw away the only record of where the view had been put, and the rule in
   * [`moved`] needs it for every scroll after the first rather than only the one
   * that follows a write. Cleared, it was -1 for exactly the events that
   * mattered: every capture of that fault carries `wrote=-1`, because the
   * handler had already run once and discarded the anchor before the growth
   * arrived.
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

  /** A wheel, a drag or a key: the reader moved it themselves. Deliberately not
   *  a touch that has not moved — see [`took`]. */
  byHand(): void {
    this.gestured = true;
  }

  /** A finger went down on the transcript. */
  took(): void {
    this.holding = true;
  }

  /** And came off it, which resumes following if they never went anywhere. */
  released(): void {
    this.holding = false;
  }

  /**
   * The view moved. Decides whether the reader is still meant to be at the end.
   *
   * ⚠ **A scroll this engine asked for is not a reader's decision**, and failing
   * to tell the two apart is what made following stop at random. The sequence,
   * measured: the view is set to the bottom and the browser queues a scroll
   * event; more of the answer renders before that event is delivered; the
   * handler then runs against the NEW `scrollHeight` and the OLD `scrollTop`,
   * computes a gap of one or two deltas' worth — 120px to 168px, where the slack
   * is 120 — and files the reader as having scrolled away. From then on nothing
   * follows, and nobody touched the screen. It bit two runs in five of the
   * deltas measurement. Remembering where we put it is the whole fix: a scroll
   * that lands exactly there is ours.
   *
   * ⚠ **Leaving and returning are not the same measurement**, and asking the
   * distance-from-the-end question for both is what kept unpinning a reader who
   * had not moved. While following, the transcript grows underneath them:
   * `scrollHeight` rises before the scroll event is handled, so the gap widens on
   * its own and crossed the 120px slack at 122, 125, 129, 130, 133 — five
   * captures, none of them a person. Their `scrollTop` never changed. So leaving
   * is measured against where we last put them, which growth cannot move;
   * returning is measured against the end, because the end is what they are
   * coming back to and it has moved since they left.
   */
  moved(box: Box): void {
    if (box.top === this.wrote) return;
    // ⚠ **A scroll with no gesture behind it is not a decision.** See
    // [`gestured`] for the measurement. The new position is taken as ours rather
    // than ignored: the browser moved it, we did not object, and treating it as
    // outstanding would make the next event look like a reader's move too.
    if (!this.gestured) {
      this.wrote = box.top;
      return;
    }
    const gap = box.height - box.top - box.view;
    this.at = this.at ? this.wrote < 0 || this.wrote - box.top < AWAY : gap < NEAR;
  }
}
