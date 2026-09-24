/**
 * Whether the newest message should be kept in view, and when to stop.
 *
 * The narrow rule every app of this kind uses: when new content arrives, if the
 * view was already at the end, keep it there. Scroll up by a line and following
 * stops; nothing ever scrolls you back.
 *
 * Never re-decide after a change whether the reader is still at the end — the
 * change itself moves the end, and no amount of slack compensates for that. The
 * one exception is [`wrote`].
 *
 * Pure and separate from the view: jsdom has no layout, so as a state machine
 * fed positions these rules are arithmetic the tests can replay.
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

/**
 * The three numbers, off a real element. Here because the shape and the reading
 * of it belong together.
 */
export function measure(box: HTMLElement): Box {
  return { top: box.scrollTop, height: box.scrollHeight, view: box.clientHeight };
}

/**
 * How near the end still counts as being at it. Under a line: scroll anchoring
 * is off for this list (see `session-view.scss`), so this covers rounding only.
 */
const SLACK = 16;

/**
 * How far a finger must travel before it is scrolling rather than resting. A
 * thumb on glass drifts a few pixels; reading back a paragraph travels
 * hundreds, so a wide range works. Near the bottom of it: a drag ignored is
 * dragged again, a page that silently stops following reads as dead.
 */
const SLOP = 40;

/** Whether a box is showing its own end. */
function atEnd(box: Box): boolean {
  return box.height - box.top - box.view < SLACK;
}

export class Following {
  /**
   * Whether the view was at the end when it was last looked at. Remembered as
   * they scroll, not measured when wanted: growth fires no scroll event, and the
   * soft keyboard takes half the screen — measured afresh then, a reader who has
   * not moved is hundreds of pixels from the bottom.
   */
  private at = true;

  /**
   * Whether a finger is on the transcript right now. A hold suspends following
   * rather than ending it: nothing is written while the finger is down, and
   * letting go catches up. A tap is a hold that lasts a moment.
   */
  private holding = false;

  /**
   * How tall the transcript was when the finger went down, or -1 for no hold, and
   * where. A hold and a drag are not the same gesture: catching up on release is
   * right for a hold and wrong for a drag. Which it was is judged against the
   * transcript they were holding, not the one they let go of — a session writes
   * throughout a hold, and against the live height every hold reads as a drag.
   */
  private heldAt: Box | undefined = undefined;

  /**
   * The last position this engine asked for, or -1 for none outstanding. The
   * view is set to the bottom, more renders before the queued scroll event is
   * delivered, and the handler then runs against the new height and the old
   * position. The event carrying exactly this position is the engine's own. Kept
   * rather than cleared: the race can follow any write.
   */
  private wrote = -1;

  /** Whether the first render has happened; before it there is nothing to keep. */
  private started = false;

  /**
   * How tall the window was at the last event, or -1 before one. A box that
   * changes shape is not a reader who moves: typing grows the composer, the gap
   * that opens is exactly what the window lost, and the soft keyboard is
   * the same thing several hundred pixels larger. Known exactly, so discounted
   * exactly rather than by widening [`SLACK`].
   */
  private view = -1;

  /**
   * Whether a move the reader did not make is expected next. See [`spoke`]. One
   * event, then spent.
   */
  private settling = false;

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
   * The last position this engine put the view at — for the trace, not a
   * decision. From outside, the engine's own write coming back and a reader's
   * scroll are the same event; this is what tells them apart.
   */
  get lastWrite(): number {
    return this.wrote;
  }

  /**
   * Where the view should be put, or `undefined` to leave it. The first
   * positioning is refused for nothing: a transcript opens at its newest message.
   */
  wants(box: Box): number | undefined {
    if (!this.started) return box.height;
    if (this.holding) return undefined;
    return this.at ? box.height : undefined;
  }

  /** Where the view actually landed — a box clamps `scrollTop`. */
  landed(top: number): void {
    this.wrote = top;
    this.started = true;
  }

  /**
   * The reader said something, and the page is about to move under them.
   *
   * This protects following; it does not restore it. Sending from halfway up the
   * morning is not a request to be taken to the bottom. What it settles is
   * a race: sending collapses the composer, and the browser moves the position
   * while it does, unasked, which from outside is
   * a reader's first scroll. What tells them apart is that one was expected.
   */
  spoke(): void {
    this.settling = this.at;
  }

  /** A finger went down on the transcript, here. */
  took(box: Box): void {
    this.holding = true;
    this.heldAt = box;
  }

  /**
   * And came off it. A hold that moved nothing leaves everything as it was; a
   * hold that scrolled is a decision, answered from where they let go against the
   * end as it stood when they took hold. See [`heldAt`].
   */
  released(box: Box): void {
    this.holding = false;
    const from = this.heldAt;
    this.heldAt = undefined;
    if (!from) return;
    // A hold, thumb drift included.
    if (Math.abs(box.top - from.top) <= SLOP) return;
    // A drag, judged against the end as it stood when they took hold. See [`heldAt`].
    this.at = atEnd({ ...box, height: from.height });
  }

  /**
   * The view moved. Whether that leaves the reader at the end is asked of where
   * they are, not how far they travelled.
   */
  moved(box: Box): void {
    // Only while following, because that is the only time anything is written —
    // see [`wrote`]; once the reader has scrolled away, a scroll landing on the last
    // written position is them coming back. A finger on the glass suspends the
    // question: a resting thumb clears [`SLACK`], and what the gesture adds up to is
    // known only when it lifts. The window is learned before every guard: it is
    // bookkeeping, and written after them the reshape reads as a first event.
    const reshaped = this.view >= 0 && box.view !== this.view;
    this.view = box.view;
    if (this.holding) return;
    if (this.at && box.top === this.wrote) return;
    // The window changed shape between this event and the last, or they have just
    // sent something and the page is re-laying out: a move the reader did not make.
    // A follower stays one, and the new position becomes the engine's own.
    if (this.at && (reshaped || this.settling)) {
      this.settling = false;
      this.wrote = box.top;
      return;
    }
    this.at = atEnd(box);
  }
}
