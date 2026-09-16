/**
 * Whether the newest message should be kept in view, and when to stop.
 *
 * A transcript that does not follow opens a hundred turns behind the present; one
 * that always follows yanks the view out from under somebody reading back.
 *
 * ⚠ **The narrow rule, which is the one every app of this kind uses:** when new
 * content arrives, if the view was already at the end, keep it at the end.
 * Scroll up by a line and following stops; nothing ever scrolls you back.
 *
 * ⚠ **Do not re-decide after a change whether the reader is still at the end.**
 * The change itself moves the end, so the measurement is contaminated by what
 * triggered it. Compensating took a gesture flag, two thresholds and 300px of
 * slack, and still pulled you back down unless you scrolled most of a screen.
 * The one piece kept from that is [`wrote`], whose race is real.
 *
 * ⚠ **Pure, and separate from the view, because these rules cannot be tested
 * where they used to live.** jsdom has no layout so a component test cannot make
 * a scroll happen, and the layout harness delivers a transcript in one chunk
 * where the runner streams it. As a state machine fed positions they are
 * arithmetic, and the numbers in these comments are what the tests replay.
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
 * ⚠ **Under a line.** A larger slack was once needed because the browser's own
 * scroll anchoring moved the position unasked; anchoring is off for this list
 * (see `session-view.scss`). What is left to cover is rounding and a stray
 * pixel, so scrolling up by one line stops the page following — which is what
 * scrolling up by one line means everywhere else on the phone.
 */
const SLACK = 16;

/**
 * How far a finger must travel before it is scrolling rather than resting.
 *
 * ⚠ **Unlike [`SLACK`] the exact value is not load-bearing.** Nothing writes a
 * scroll position while a finger is down, so everything the view does during a
 * hold is the hand doing it — and the two hands are an order of magnitude apart:
 * a thumb resting on glass drifts a few pixels over a long hold, while reading
 * back a paragraph travels hundreds. A wide range of thresholds sorts both.
 *
 * Deliberately near the bottom of that range. Too high and a small deliberate
 * drag is ignored, so the reader drags again; too low and the page silently
 * stops following, which reads as dead. Those costs are not comparable.
 */
const SLOP = 40;

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
   * How tall the transcript was when the finger went down, or -1 for no hold.
   *
   * ⚠ **A hold and a drag are not the same gesture, and treating them alike put
   * the view back at the end the moment somebody let go of a scroll.** Catching
   * up on release is right for a hold — they stopped the page to read a line and
   * then let it go — and wrong for a drag, where letting go is simply the end of
   * the scroll they just performed.
   *
   * ⚠ **Which of the two it was is asked of the transcript they were holding,
   * not the one they let go of**, and that is this field's whole reason to
   * exist. A hold lasts seconds and a session writes throughout it, so by the
   * time the finger lifts, the end has run hundreds of pixels away from a reader
   * who never moved. Judged against the live height, every hold longer than a
   * moment reads as a drag — [`moved`]'s contamination, arriving on the one path
   * that had not been fixed for it.
   *
   * The position is kept for the same reason the height is: what the gesture
   * came to is the distance between where the finger went down and where it came
   * up, and that is the quantity [`SLOP`] is asked about. Neither is knowable
   * from the events in between — a drag arrives as thirty small ones, and the
   * first of them is indistinguishable from a thumb.
   */
  private heldAt: Box | undefined = undefined;

  /**
   * The last position this engine asked for, or -1 for none outstanding.
   *
   * ⚠ **The one piece of the old machinery still needed.** The view is set to
   * the bottom and the browser queues a scroll event; more of the answer renders
   * before that event is delivered; the handler then runs against the NEW height
   * and the OLD position, and reads a delta's worth of gap as a reader walking
   * away. The position carried by that event is exactly where this engine put
   * it, which is what tells the two apart.
   *
   * Kept rather than cleared once used: the race can follow any write, not only
   * the first.
   */
  private wrote = -1;

  /** Whether the first render has happened; before it there is nothing to keep. */
  private started = false;

  /**
   * How tall the window was at the last event, or -1 before there has been one.
   *
   * ⚠ **A box that changes shape is not a reader who moves, and [`wrote`] cannot
   * see the difference.** Typing grows the composer, which takes height from the
   * transcript, and the gap that opens is exactly what the window lost — the
   * reader has not moved (#731). But the reshape shifts `top` too, so the
   * `wrote` guard cannot recognise it and `atEnd` is asked about a window that is
   * no longer the one the answer was true of.
   *
   * The soft keyboard is the same thing several hundred pixels larger, which is
   * why this must not be answered by widening [`SLACK`]: the quantity is known
   * exactly, so it is discounted exactly.
   */
  private view = -1;

  /**
   * Whether a move the reader did not make is expected next. See [`spoke`].
   *
   * One event, then spent: the shift arrives as a single scroll after the send,
   * and re-anchoring [`wrote`] to where it lands is what stops the settling event
   * behind it re-opening the question. A flag that outlived its relayout would
   * forgive the reader's next real scroll instead.
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
   * The last position this engine put the view at — for the trace, not for a
   * decision.
   *
   * ⚠ **Exposed because reasoning about it from the source failed twice.** A
   * scroll that carries exactly this position is the engine's own write coming
   * back (see [`wrote`]); one that does not is somebody or something else. From
   * outside, the two are the same event, and the console's trace could not tell
   * them apart — so an unpin every fifteen seconds had no explanation and got a
   * guessed one, which was wrong.
   */
  get lastWrite(): number {
    return this.wrote;
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

  /**
   * The reader said something, and the page is about to move under them.
   *
   * ⚠ **This PROTECTS following; it does not restore it** — Pippijn's rule, and
   * the right one. Sending from halfway up the morning is not a request to be
   * taken to the bottom: the message goes to the end whether or not it is
   * watched, and yanking the view is the very thing #82 exists to prevent. So a
   * reader who had scrolled away stays exactly where they are.
   *
   * What it does settle is a race that cannot be won by measuring. Sending
   * re-lays the page out — the composer collapses from four lines to one — and
   * the browser moves the position while it does: measured on the phone (#731),
   * `top` went 145157 to 145066 with no finger down and `was == wrote`, i.e. 91px
   * nobody asked for, and the transcript detached 110ms after the tap. From
   * outside, that is the same event as a reader's first scroll. What tells them
   * apart is not the numbers but that one of them was expected.
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
   * And came off it.
   *
   * A hold that moved nothing leaves everything as it was, so a reader who was
   * following still is and the view catches up. A hold that *scrolled* is a
   * decision about where to be, and is answered from where they let go —
   * measured against the end as it stood when they took hold. See [`heldAt`].
   */
  released(box: Box): void {
    this.holding = false;
    const from = this.heldAt;
    this.heldAt = undefined;
    if (!from) return;
    // A hold. Nothing about where the reader wants to be has changed, and that
    // includes a thumb that dragged the page a few pixels while it rested.
    if (Math.abs(box.top - from.top) <= SLOP) return;
    // A drag, answered against the end as it stood when they took hold: a
    // session writing throughout a long hold moves the end, and that is not the
    // reader travelling away from it. See [`heldAt`].
    this.at = atEnd({ ...box, height: from.height });
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
    // ⚠ **A finger on the glass suspends the question, it does not answer it.**
    // A reader holding still and not scrolling still moves the view a few
    // pixels, which is enough to clear [`SLACK`] and stop following for good.
    // That movement is a thumb on glass, not a decision, and no threshold tells
    // the two apart from a single event — what does is what the gesture adds up
    // to, which is not known until the finger lifts. See [`heldAt`],
    // [`released`].
    // ⚠ **Before every guard, because it is bookkeeping and not a decision.**
    // Written after the guards first, where the `wrote` short-circuit meant the
    // window was never learned on the events that took it — so the reshape that
    // followed looked like the first event this engine had ever seen and was not
    // recognised at all.
    const reshaped = this.view >= 0 && box.view !== this.view;
    this.view = box.view;
    if (this.holding) return;
    if (this.at && box.top === this.wrote) return;
    // The window changed shape between this event and the last. Whatever gap
    // that accounts for belongs to the composer or the keyboard, not to the
    // reader — so a follower stays one, and the new position becomes what the
    // engine considers its own, which is what stops the settling event after it
    // reading as somebody walking away. See [`view`].
    // A move the reader did not make: the window changed shape, or they have
    // just sent something and the page is re-laying out around it.
    if (this.at && (reshaped || this.settling)) {
      this.settling = false;
      this.wrote = box.top;
      return;
    }
    this.at = atEnd(box);
  }
}
