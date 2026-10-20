/**
 * Whether the newest message should be kept in view, and when to stop: if the
 * view was at the end when content arrives, keep it there; scroll up by a line
 * and following stops, and nothing scrolls you back.
 *
 * Never re-decide after a change whether the reader is at the end — the change
 * itself moves the end. The one exception is [`wrote`].
 *
 * A state machine fed positions, apart from the view, because jsdom has no layout.
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

export function measure(box: HTMLElement): Box {
  return { top: box.scrollTop, height: box.scrollHeight, view: box.clientHeight };
}

/**
 * How near the end still counts as at it: rounding only, since scroll anchoring
 * is off for this list (see `session-view.scss`).
 */
const SLACK = 16;

/**
 * How far a finger travels before it is scrolling, not resting. Low in the safe
 * range: an ignored drag is dragged again, while a page that silently stops
 * following reads as dead.
 */
const SLOP = 40;

function atEnd(box: Box): boolean {
  return box.height - box.top - box.view < SLACK;
}

export class Following {
  /**
   * Whether the view was at the end when last looked at. Remembered, not
   * measured when wanted: growth fires no scroll event, and with the soft
   * keyboard up a reader who has not moved is hundreds of pixels from the bottom.
   */
  private at = true;

  /**
   * A finger is on the transcript. A hold suspends following rather than ending
   * it: nothing is written while it is down, and letting go catches up.
   */
  private holding = false;

  /**
   * The box when the finger went down. Hold or drag is judged against the
   * transcript as it was held: a session writes throughout a hold, and against the
   * live height every hold reads as a drag.
   */
  private heldAt: Box | undefined = undefined;

  /**
   * The last position this engine asked for, or -1. More renders before the queued
   * scroll event arrives, so the handler sees the new height and the old position;
   * the event carrying exactly this position is the engine's own. Kept, not
   * cleared: the race can follow any write.
   */
  private wrote = -1;

  private started = false;

  /**
   * The window's height at the last event, or -1. A box changing shape is not a
   * reader moving: the gap the composer or the keyboard opens is exactly what the
   * window lost, so it is discounted exactly rather than by widening [`SLACK`].
   */
  private view = -1;

  /** A move the reader did not make is expected next — see [`spoke`]. One event. */
  private settling = false;

  get pinned(): boolean {
    return this.at;
  }

  get settled(): boolean {
    return this.started;
  }

  get held(): boolean {
    return this.holding;
  }

  /**
   * For the trace: from outside, the engine's own write coming back and a
   * reader's scroll are the same event.
   */
  get lastWrite(): number {
    return this.wrote;
  }

  /**
   * Where to put the view, or `undefined` to leave it. A transcript opens at its
   * newest message.
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
   * The reader sent something. This protects following; it does not restore it:
   * sending collapses the composer and the browser moves the position unasked,
   * which from outside is a reader's scroll, except that it was expected.
   */
  spoke(): void {
    this.settling = this.at;
  }

  took(box: Box): void {
    this.holding = true;
    this.heldAt = box;
  }

  /**
   * A hold that moved nothing changes nothing; one that scrolled decides, from
   * where they let go against the end as it was when they took hold.
   */
  released(box: Box): void {
    this.holding = false;
    const from = this.heldAt;
    this.heldAt = undefined;
    if (!from) return;
    // A hold, thumb drift included.
    if (Math.abs(box.top - from.top) <= SLOP) return;
    this.at = atEnd({ ...box, height: from.height });
  }

  /** The view moved. Asked of where the reader is, not how far they travelled. */
  moved(box: Box): void {
    // Learned before every guard: it is bookkeeping, and after them a reshape
    // reads as a first event.
    const reshaped = this.view >= 0 && box.view !== this.view;
    const lost = reshaped ? Math.max(0, this.view - box.view) : 0;
    this.view = box.view;
    // What the gesture adds up to is known only when the finger lifts.
    if (this.holding) return;
    // Only while following is anything written; once they have scrolled away, a
    // scroll onto the last written position is them coming back.
    if (this.at && box.top === this.wrote) return;
    // A reshape, or the page re-laying out after a send: a follower stays one. A
    // reshape opens a gap no wider than what the window lost; past that it is the
    // reader, scrolling in the same frame.
    const reshapeExplains = reshaped && box.height - box.top - box.view < lost + SLACK;
    if (this.at && (reshapeExplains || this.settling)) {
      this.settling = false;
      this.wrote = box.top;
      return;
    }
    this.at = atEnd(box);
  }
}
