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
   * Background tool calls started and not yet reported finished, by tool id.
   *
   * ⚠ **Only the ones the harness tracks.** A command backgrounded inside a
   * shell — `nohup … &` — returns at once and announces nothing, so it is
   * invisible here. This counts what it can see and the label says so, because
   * a bare "nothing running" would be a claim this cannot support.
   */
  readonly background: WritableSignal<string[]>;
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
      background: signal<string[]>([]),
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
    // A new process cannot have inherited the last one's background work: the
    // tasks died with it, and their notifications died with them. Without this
    // a console restart leaves phantoms in the count for ever — measured, at 11.
    if (event.kind === 'started') held.background.set([]);
    if (event.kind === 'tool' && event.input?.['run_in_background'] === true && event.id) {
      const id = event.id;
      held.background.update((running) => (running.includes(id) ? running : [...running, id]));
    }
    if (event.kind === 'background' && event.tool) {
      const done = event.tool;
      held.background.update((running) => running.filter((id) => id !== done));
    }
    if (event.kind === 'busy') held.doing.set(event.status ?? 'working');
    if (event.kind === 'turn' || event.kind === 'exited') held.doing.set(undefined);
    held.entries.update((entries) => [...fold(entries, event)]);
  }

  /**
   * Drop everything held about a conversation.
   *
   * All three, because they are one fact in three places: the entries, where the
   * page begins — which the `joined` event of the replay re-establishes — and
   * how far the transcript had got, which is now nowhere.
   */
  private forget(held: Held): void {
    held.entries.set([]);
    held.cursor.set(0);
    held.seen = 0;
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
