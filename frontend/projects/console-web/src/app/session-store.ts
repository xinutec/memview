import { Injectable, WritableSignal, inject, signal } from '@angular/core';
import { Observable, map } from 'rxjs';

import { ConsoleApi } from './console-api';
import { unhandled } from './exhaustive';
import { Kept } from './kept';
import { Outbox } from './outbox';
import { type Change, Entry, Timed } from './models';
import { reason } from './errors';
import type { Hunk } from './generated/Hunk';
import { Patience } from './patience';
import { fold } from './transcript';

/**
 * How many sessions' transcripts are kept once off screen. Small: the memory
 * of the last few things looked at, not a cache of everything.
 */
const KEPT = 4;

/** One session's transcript, and the reader's place in it. */
export interface Held {
  readonly entries: WritableSignal<Entry[]>;
  /** Where the page on screen begins in the transcript, as a byte offset. */
  readonly cursor: WritableSignal<number>;
  /**
   * What the session is doing right now, or undefined when idle. Live, from the
   * stream's `busy` events, which the transcript deliberately drops.
   */
  readonly doing: WritableSignal<string | undefined>;
  /**
   * When it started working, in milliseconds, for the timer beside [doing] — set
   * on leaving idle, from the event's own stamp.
   */
  readonly since: WritableSignal<number | undefined>;
  /**
   * Whether the stream has said anything about activity since this transcript was
   * seeded — what makes [doing]'s `undefined` mean "idle". Until it has,
   * `undefined` means "no idea" and the summary's `busy` is the better answer.
   */
  readonly spoken: WritableSignal<boolean>;
  /**
   * Whether the stream is past the replay — the runner's `caught-up` event. Not
   * [spoken], which is whether the present has been described.
   */
  readonly live: WritableSignal<boolean>;
  /**
   * Where what is on screen came from. `kept` is the copy this phone last saw,
   * shown from opening until the conversation has wholly arrived — and never
   * written back. See [[Kept]].
   */
  readonly source: WritableSignal<'stream' | 'kept'>;
  /** The stream has failed since the copy went up, so it is a copy and not a wait. */
  readonly unanswered: WritableSignal<boolean>;
  /** The conversation arriving behind a kept copy, swapped in when the replay ends. */
  building?: Entry[];
  /**
   * Whether the stream has been down long enough to say so — see [[Patience]]. A
   * dead stream otherwise looks like a session thinking.
   */
  readonly link: Patience;
  /**
   * Whether the reader has jumped away from the live end. Only
   * [[SessionStore.goTo]] sets this and only [[SessionStore.rejoin]] clears it: it
   * is about where the reader is, not the connection.
   */
  readonly adrift: WritableSignal<boolean>;
  /** What each `Bash` call is predicted to change, by call — see [[ConsoleApi.edits]]. */
  readonly edited: WritableSignal<ReadonlyMap<string, readonly Change[]>>;
  /** Calls whose files did not end up as predicted. */
  readonly diverged: WritableSignal<ReadonlySet<string>>;
  // No background count here: it arrives on the summary, from the runner, which is
  // the copy that survives a reload. See `session::Summary::background`.
  /** The last sequence number this transcript accounts for, 0 for none. */
  seen: number;
  /** Closes the stream, while there is one. */
  close?: () => void;
  /** Ordering for eviction: higher is more recently opened. */
  used: number;
}

/**
 * The transcripts being read, and the streams that fill them. Root-provided, so
 * a navigation keeps what was read; a stream is closed on leaving, since browsers
 * allow few connections per host.
 */
@Injectable({ providedIn: 'root' })
export class SessionStore {
  private api = inject(ConsoleApi);
  private kept = inject(Kept);
  private outbox = inject(Outbox);
  private held = new Map<string, Held>();
  private clock = 0;

  /**
   * Read a session: resume the transcript if it is still here, start it if not. An
   * id opened twice ends with one stream.
   */
  open(id: string): Held {
    const held = this.held.get(id) ?? this.fresh(id);
    held.close?.();
    held.used = ++this.clock;
    this.held.set(id, held);
    // What was seen last time, at once: the Mac's answer replaces it.
    void this.hydrate(id, held);
    // Kept on the Mac, so it covers calls older than the stream's scrollback.
    // Merged, never set: the record and the stream race, and an edit the stream
    // announced before the record landed must survive it.
    this.api.edits(id).subscribe({
      next: (record) => {
        held.edited.update((edited) => {
          const merged = new Map(edited);
          for (const one of record.edited) merged.set(one.call, one.hunks.map(change));
          return merged;
        });
        held.diverged.update(
          (diverged) => new Set([...diverged, ...record.diverged.map((one) => one.call)]),
        );
      },
      error: (err: unknown) => console.warn('edits:', reason(err)),
    });
    const watching = this.api.follow(id, held.seen).subscribe((from) => {
      switch (from.kind) {
        case 'event':
          held.link.right();
          held.unanswered.set(false);
          this.take(id, held, from.event, from.seq);
          break;
        // Only when the runner says the stream starts again — see [[ConsoleApi]].
        // Everything held has to go, or a replay would append to a copy of itself.
        case 'reset':
          this.forget(held);
          break;
        // The replay is over; what follows is happening — see [Held.live].
        case 'caught-up':
          held.link.right();
          held.live.set(true);
          if (held.source() === 'kept') {
            held.entries.set(held.building ?? []);
            held.building = undefined;
            held.source.set('stream');
            held.unanswered.set(false);
          }
          break;
        case 'offline':
          // Timed from here and cancelled by the next event, so a stream that keeps
          // reconnecting never reaches the banner.
          held.link.wrong({ kind: 'stream' });
          held.unanswered.set(true);
          break;
        default:
          unhandled(from);
      }
    });
    held.close = () => watching.unsubscribe();
    this.evict();
    return held;
  }

  /** Show what was last kept, while nothing of the conversation has arrived. See [[Kept]]. */
  private async hydrate(id: string, held: Held): Promise<void> {
    const copy = await this.kept.entries(id);
    if (!copy.length || held.entries().length || held.seen > 0) return;
    held.entries.set(copy);
    held.source.set('kept');
  }

  /**
   * Stop reading, without forgetting: the entries, the cursor and the sequence
   * number stay, which is what makes coming back cheap.
   */
  leave(id: string): void {
    const held = this.held.get(id);
    if (!held) return;
    held.close?.();
    held.close = undefined;
    // Not reading it any more, so a drop is nothing to report.
    held.link.right();
    // Flushed on the way out, past the throttle: leaving is the moment a copy is
    // most likely to be wanted next.
    if (held.source() === 'stream') this.kept.keepNow(id, held.entries());
  }

  /**
   * Put the page before the one on screen in front of it. Reports nothing about
   * scrolling: holding the reader's place is a DOM measurement the caller has.
   */
  earlier(id: string): Observable<void> {
    const held = this.held.get(id) ?? this.fresh(id);
    return this.api.earlier(id, held.cursor()).pipe(
      map((older) => {
        held.cursor.set(older.from);
        // Folded on their own and put in front: fold joins an event to what precedes
        // it, and appending would glue the top of the conversation onto the bottom.
        let head: Entry[] = [];
        for (const event of older.events) head = fold(head, event);
        held.entries.update((entries) => [...head, ...entries]);
      }),
    );
  }

  /**
   * Show the page that ends at a landmark, closing the live stream first so
   * nothing new lands under an old page.
   */
  goTo(id: string, at: number): Observable<void> {
    const held = this.held.get(id) ?? this.fresh(id);
    held.close?.();
    held.close = undefined;
    return this.api.earlier(id, at).pipe(
      map((there) => {
        let page: Entry[] = [];
        for (const event of there.events) page = fold(page, event);
        held.entries.set(page);
        // From the Mac, so whatever copy was up has been answered.
        held.source.set('stream');
        held.building = undefined;
        held.unanswered.set(false);
        held.cursor.set(there.from);
        held.adrift.set(true);
        // Nothing is known about the present any more — the stream that would have
        // said is closed.
        held.live.set(false);
        held.spoken.set(false);
        held.doing.set(undefined);
        held.since.set(undefined);
      }),
    );
  }

  /**
   * Come back to the present from a jump. Everything held goes: it is a page from
   * the middle of the file, and the stream about to arrive replays the end.
   */
  rejoin(id: string): Held {
    const held = this.held.get(id) ?? this.fresh(id);
    this.forget(held);
    held.adrift.set(false);
    return this.open(id);
  }

  private fresh(id: string): Held {
    const held: Held = {
      entries: signal<Entry[]>([]),
      cursor: signal(0),
      doing: signal<string | undefined>(undefined),
      since: signal<number | undefined>(undefined),
      spoken: signal(false),
      live: signal(false),
      source: signal<'stream' | 'kept'>('stream'),
      unanswered: signal(false),
      link: new Patience(),
      adrift: signal(false),
      edited: signal<ReadonlyMap<string, readonly Change[]>>(new Map()),
      diverged: signal<ReadonlySet<string>>(new Set()),
      seen: 0,
      used: ++this.clock,
    };
    this.held.set(id, held);
    return held;
  }

  private take(id: string, held: Held, event: Timed, seq: number): void {
    // The seed arrives with the cursor it started from — the only place that learns
    // where the page on screen begins.
    if (event.kind === 'joined') held.cursor.set(event.from);
    // Only ever forward: unnumbered events arrive as 0.
    if (seq > held.seen) held.seen = seq;
    // Activity is state beside the transcript, from live events only: a seed's
    // closing `turn` would show `idle` over a session running tools.
    if (held.live()) {
      if (event.kind === 'busy') {
        // Only the first one starts the clock — see [Held.since].
        if (held.doing() === undefined) held.since.set(event.at ?? Date.now());
        held.doing.set(event.status);
        held.spoken.set(true);
      }
      if (event.kind === 'turn' || event.kind === 'exited') {
        held.doing.set(undefined);
        held.since.set(undefined);
        held.spoken.set(true);
      }
    }
    if (event.kind === 'edited') {
      const hunks = event.hunks.map(change);
      held.edited.update((edited) => new Map(edited).set(event.call, hunks));
    }
    if (event.kind === 'diverged') {
      held.diverged.update((diverged) => new Set(diverged).add(event.call));
    }
    // Where a message of this phone's stands — see [[Outbox]].
    if (event.kind === 'accepted' && event.id) this.outbox.accepted(event.id);
    if ((event.kind === 'prompt' || event.kind === 'shown') && event.id) this.outbox.done(event.id);
    if (event.kind === 'withdrawn') this.outbox.done(event.id);
    // Behind a kept copy the conversation is built out of sight: a page half
    // arrived over a slow link would replace the copy with less than it had.
    if (held.source() === 'kept') held.building = fold(held.building ?? [], event);
    else held.entries.update((entries) => fold(entries, event));
    // Throttled inside, and not done on leaving instead: a phone stops reading when
    // the tunnel drops or the app is killed, and neither runs code here.
    if (held.source() === 'stream') this.kept.keep(id, held.entries());
  }

  /**
   * Drop everything held about a conversation, its activity included, or a turn
   * that ended unseen leaves the timer running.
   */
  private forget(held: Held): void {
    held.link.right();
    // A kept copy stays up; what was being built behind it starts again.
    if (held.source() === 'kept') held.building = undefined;
    else held.entries.set([]);
    held.cursor.set(0);
    held.seen = 0;
    held.doing.set(undefined);
    held.since.set(undefined);
    held.spoken.set(false);
    held.live.set(false);
  }

  /**
   * Let go of the least recently opened transcripts past [KEPT], never one with a
   * stream on it.
   */
  private evict(): void {
    const idle = [...this.held.entries()]
      .filter(([, held]) => !held.close)
      .sort((a, b) => a[1].used - b[1].used);
    for (const [id] of idle.slice(0, Math.max(0, this.held.size - KEPT))) {
      this.held.delete(id);
    }
  }
}

/** An observed hunk, in the shape the diff view draws. Only the lines shown changed. */
function change(hunk: Hunk): Change {
  return { ...hunk, everywhere: false };
}
