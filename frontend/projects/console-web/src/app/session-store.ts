import { Injectable, WritableSignal, inject, signal } from '@angular/core';
import { Observable, map } from 'rxjs';

import { ConsoleApi } from './console-api';
import { Entry, SessionEvent } from './models';
import { fold } from './transcript';

/**
 * How many sessions' transcripts are kept once they are no longer on screen.
 *
 * Small on purpose. This is the memory of the last few things you looked at, not
 * a cache of everything — the transcripts are on disk, the runner will replay
 * them, and a phone is not the place to hold a dozen conversations that nobody
 * is reading.
 */
const KEPT = 4;

/** One session's transcript, and the reader's place in it. */
export interface Held {
  readonly entries: WritableSignal<Entry[]>;
  /** Where the page on screen begins in the transcript, as a byte offset. */
  readonly cursor: WritableSignal<number>;
  /**
   * What the session is doing right now, or undefined when it is idle.
   *
   * Live, from the stream's own `busy` events, which the transcript
   * deliberately drops — they are state, not conversation. Before this they
   * were dropped by everybody, so the only source of "is it working" was the
   * five-second poll of the summary: too slow to catch a short burst and
   * silent for up to five seconds at the start of a long one.
   */
  readonly doing: WritableSignal<string | undefined>;
  /**
   * When it started working, in milliseconds — for the timer beside [doing].
   *
   * ⚠ **Set when it stops being idle, not on every status.** The CLI reports
   * several statuses inside one stretch of work, and restarting the count on
   * each would answer "how long has it been *requesting*" — which is a question
   * about the CLI's vocabulary. The one worth asking is how long you have been
   * waiting, and that runs from the moment it stopped being idle.
   *
   * From the event's own stamp rather than the clock, so a burst replayed after
   * a reconnect is not dated to the reconnection.
   */
  readonly since: WritableSignal<number | undefined>;
  /**
   * Whether the stream has said anything about activity since this transcript
   * was seeded — which is what makes [doing]'s `undefined` mean "idle".
   *
   * ⚠ **Until it has, `undefined` means "no idea" instead**, and the summary's
   * own `busy` is the better answer. A re-seed leaves this client knowing
   * nothing about the present: the console announces a status when it changes,
   * so a session that was already working before the reconnect says nothing
   * further until it stops. Without this the page read `idle` over a session
   * that was plainly working, having simply not been listening when it said so.
   */
  readonly spoken: WritableSignal<boolean>;
  // ⚠ **No background count here, and there was one.** It was derived from this
  // stream, which was the only way to know until the runner started counting
  // for the list — and then the same question had two answers: this one reset
  // whenever the transcript was re-seeded and the runner's did not. It now
  // arrives on the summary, from the runner, which is the copy that survives a
  // reload. See `session::Summary::background` and `protocol::running`.
  /** The last sequence number this transcript accounts for, 0 for none. */
  seen: number;
  /** Closes the stream, while there is one. */
  close?: () => void;
  /** Ordering for eviction: higher is more recently opened. */
  used: number;
}

/**
 * The transcripts being read, and the streams that fill them.
 *
 * ⚠ **Root-provided because the component is not the reader — the person is.**
 * Held in the component, the fold and its stream died with the view, so opening
 * a session, going back to the list and opening it again started from nothing:
 * every page somebody had scrolled back to load was thrown away by a navigation.
 * That is the same defect a dropped connection used to have, and it has the same
 * fix — say what you hold and be sent the rest.
 *
 * The stream is closed on the way out rather than left running. A session nobody
 * is looking at has nothing to say to the screen, and browsers allow only a
 * handful of connections to one host — a few abandoned streams would starve the
 * state poll, which is the part that says whether the Mac is reachable at all.
 * Closing costs nothing now that re-opening resumes.
 *
 * Deliberately not kept: where the reader was scrolled to. Restoring an offset
 * measured against a transcript that has grown since lands somewhere arbitrary,
 * and entries carry no identity to anchor on instead. Re-entry goes to the
 * newest message, which is at least always the same answer.
 */
@Injectable({ providedIn: 'root' })
export class SessionStore {
  private api = inject(ConsoleApi);
  private held = new Map<string, Held>();
  private clock = 0;

  /**
   * Read a session: resume the transcript if it is still here, start it if not.
   *
   * Idempotent in the way that matters — an id opened twice ends with one
   * stream, because the second open closes whatever the first left running.
   */
  open(id: string): Held {
    const held = this.held.get(id) ?? this.fresh(id);
    held.close?.();
    held.used = ++this.clock;
    this.held.set(id, held);
    held.close = this.api.follow(
      id,
      held.seen,
      (event, seq) => this.take(held, event, seq),
      // Only when the runner says the stream starts again — see [[ConsoleApi]].
      // Everything held has to go: it would otherwise be appended to by a replay
      // of itself, and there is no way to tell the two copies apart.
      () => this.forget(held),
    );
    this.evict();
    return held;
  }

  /**
   * Stop reading, without forgetting.
   *
   * The transcript stays; only the stream goes. What survives here is exactly
   * what makes coming back cheap: the entries, the cursor into the file, and the
   * sequence number to resume from.
   */
  leave(id: string): void {
    const held = this.held.get(id);
    if (!held) return;
    held.close?.();
    held.close = undefined;
  }

  /**
   * Put the page before the one on screen in front of it.
   *
   * The store does this rather than the view because the entries are the store's
   * — but it deliberately reports nothing about scrolling. Holding the reader's
   * place is a measurement of the DOM, and the caller is the one that has it.
   */
  earlier(id: string): Observable<void> {
    const held = this.held.get(id) ?? this.fresh(id);
    return this.api.earlier(id, held.cursor()).pipe(
      map((older) => {
        held.cursor.set(older.from);
        // Folded on their own and put in front, rather than folded into the
        // list: fold joins an event to whatever precedes it, and an older page
        // has nothing before it here — appending would glue the top of the
        // conversation onto the bottom.
        let head: Entry[] = [];
        for (const event of older.events) head = [...fold(head, event)];
        held.entries.update((entries) => [...head, ...entries]);
      }),
    );
  }

  private fresh(id: string): Held {
    const held: Held = {
      entries: signal<Entry[]>([]),
      cursor: signal(0),
      doing: signal<string | undefined>(undefined),
      since: signal<number | undefined>(undefined),
      spoken: signal(false),
      seen: 0,
      used: ++this.clock,
    };
    this.held.set(id, held);
    return held;
  }

  private take(held: Held, event: SessionEvent, seq: number): void {
    // The seed arrives with the cursor it started from. This is the only place
    // that learns where the page on screen begins — nothing else in the stream
    // knows the conversation is longer than the page.
    if (event.kind === 'joined') held.cursor.set(event.from ?? 0);
    // Only ever forward. The unnumbered events arrive as 0, and a transcript
    // that claimed to hold nothing after one of those would ask for the whole
    // conversation again on the next reconnect.
    if (seq > held.seen) held.seen = seq;
    // Activity is state, so it is kept beside the transcript rather than in it.
    // A turn ending is what says the work stopped: the runner clears its own
    // busy on the same event, and nothing else on the wire announces idleness.
    if (event.kind === 'busy') {
      // Only the first one starts the clock — see [Held.since].
      if (held.doing() === undefined) held.since.set(event.at ?? Date.now());
      held.doing.set(event.status ?? 'working');
      held.spoken.set(true);
    }
    if (event.kind === 'turn' || event.kind === 'exited') {
      held.doing.set(undefined);
      held.since.set(undefined);
      held.spoken.set(true);
    }
    held.entries.update((entries) => [...fold(entries, event)]);
  }

  /**
   * Drop everything held about a conversation.
   *
   * All three, because they are one fact in three places: the entries, where the
   * page begins — which the `joined` event of the replay re-establishes — and
   * how far the transcript had got, which is now nowhere.
   *
   * ⚠ **What it was doing goes too, and that is the point.** [Held.doing] is
   * cleared only by the `turn` or `exited` that ends the work, so a turn that
   * ended while this client was disconnected clears nothing: the console
   * replaced itself mid-turn, the event went to nobody, and the page showed a
   * session working with a timer running for as long as it was left open. A
   * re-seed means this client knows nothing about the present, which is what
   * `undefined` says — the next status line off the stream says the rest.
   */
  private forget(held: Held): void {
    held.entries.set([]);
    held.cursor.set(0);
    held.seen = 0;
    held.doing.set(undefined);
    held.since.set(undefined);
    held.spoken.set(false);
  }

  /** Let go of the least recently opened transcripts past [KEPT].
   *
   *  Never one with a stream on it: that is a session being read right now, and
   *  the count is of what is being remembered rather than what is being used. */
  private evict(): void {
    const idle = [...this.held.entries()]
      .filter(([, held]) => !held.close)
      .sort((a, b) => a[1].used - b[1].used);
    for (const [id] of idle.slice(0, Math.max(0, this.held.size - KEPT))) {
      this.held.delete(id);
    }
  }
}
