import { TestBed } from '@angular/core/testing';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Observable, of } from 'rxjs';

import { ConsoleApi, type Streamed } from './console-api';
import { Local } from './local';
import { Entry, Page, Timed } from './models';
import { SessionStore } from './session-store';
import { first, last, nth } from './testing';

/** One stream the store opened, and the handles to drive it from the test. */
interface Opened {
  readonly id: string;
  /** What the store said it already held. Zero means "nothing, send everything". */
  readonly after: number;
  readonly send: (event: Timed, seq: number) => void;
  readonly reset: () => void;
  /** The runner's per-connection marker: the replay is over. */
  readonly caughtUp: () => void;
  /** The tunnel dropping. The browser retries, so this is "not right now". */
  readonly offline: () => void;
  closed: boolean;
}

/**
 * A runner that never answers unless the test tells it to.
 *
 * The point of these tests is what the *store* says on the way in, so the stub
 * records the request and hands back the levers rather than simulating a server.
 */
class Runner {
  readonly opened: Opened[] = [];

  follow(id: string, after: number): Observable<Streamed> {
    return new Observable<Streamed>((to) => {
      const stream: Opened = {
        id,
        after,
        send: (event, seq) => to.next({ kind: 'event', event, seq }),
        reset: () => to.next({ kind: 'reset' }),
        caughtUp: () => to.next({ kind: 'caught-up' }),
        offline: () => to.next({ kind: 'offline' }),
        closed: false,
      };
      this.opened.push(stream);
      return () => (stream.closed = true);
    });
  }

  /** What a page fetched by cursor comes back with. Set by the test. */
  page: Page = { events: [], from: 0 };
  /** The cursors asked for, in order. */
  readonly asked: number[] = [];

  earlier(_id: string, before: number): Observable<Page> {
    this.asked.push(before);
    return of(this.page);
  }

  /** The stream opened most recently, which is the one under test. */
  get latest(): Opened {
    return last(this.opened);
  }
}

const said = (entries: readonly Entry[]): string[] =>
  entries.flatMap((entry) => (entry.kind === 'said' ? [entry.text] : []));

describe('SessionStore', () => {
  let runner: Runner;
  let store: SessionStore;

  beforeEach(() => {
    runner = new Runner();
    TestBed.configureTestingModule({
      providers: [{ provide: ConsoleApi, useValue: runner }],
    });
    // A database of this test's own, before anything reaches for it: [[Kept]]
    // keeps the offline copy in it, and two tests sharing one would read each
    // other's transcripts.
    TestBed.inject(Local).under(`t${Math.random().toString(36).slice(2)}`);
    store = TestBed.inject(SessionStore);
  });

  afterEach(async () => {
    TestBed.inject(Local).close();
    await Promise.resolve();
  });

  /** Say something, as the runner would, and number it. */
  function say(text: string, seq: number): void {
    runner.latest.send({ kind: 'text', text }, seq);
    runner.latest.send({ kind: 'turn', cost_usd: 0, turns: 1, duration_ms: 0 }, seq + 1);
  }

  it('resumes a session it is re-entered rather than reading it again', () => {
    // The whole reason this store exists. Held in the view, the transcript died
    // with it, so going back to the list and in again threw away every page
    // somebody had scrolled back to load — silently, and looking like a reload.
    const held = store.open('one');
    expect(runner.latest.after).toBe(0);
    say('an answer', 10);
    expect(said(held.entries())).toContain('an answer');

    store.leave('one');
    expect(first(runner.opened).closed).toBe(true);

    const again = store.open('one');
    expect(again).toBe(held);
    expect(said(again.entries())).toContain('an answer');
    expect(runner.latest.after).toBe(11);
  });

  it('asks for everything when it has nothing, and only then', () => {
    store.open('fresh');
    expect(runner.latest.after).toBe(0);
    // Numbered events move it forward; the unnumbered ones the runner sends when
    // it has dropped events must not move it back, or the next reconnect would
    // ask for the conversation from the top.
    say('first', 4);
    runner.latest.send({ kind: 'trouble', detail: 'dropped' }, 0);
    store.leave('fresh');
    store.open('fresh');
    expect(runner.latest.after).toBe(5);
  });

  it('empties what it holds when the runner says the stream starts again', () => {
    // The one case where keeping the transcript is wrong: a replay would be
    // appended to a copy of itself, and the two are indistinguishable.
    const held = store.open('restarted');
    say('before', 8);
    expect(held.entries().length).toBeGreaterThan(0);

    runner.latest.reset();
    expect(held.entries()).toEqual([]);

    store.leave('restarted');
    store.open('restarted');
    expect(runner.latest.after).toBe(0);
  });

  it('opening a session twice leaves one stream running', () => {
    store.open('twice');
    store.open('twice');
    expect(runner.opened.length).toBe(2);
    expect(first(runner.opened).closed).toBe(true);
    expect(nth(runner.opened, 1).closed).toBe(false);
  });

  it('lets go of what it was doing when the stream starts again', () => {
    // ⚠ **A turn that ends during a reconnect ends for nobody.** `doing` is
    // cleared by the `turn` event, so a console that replaced itself mid-turn
    // left the page showing a session working, timer running, for as long as it
    // stayed open — while the front page, reading the runner's own flag, said
    // idle. A re-seed means this client knows nothing about the present, and
    // `spoken` is how it says so rather than claiming the session is idle.
    const held = store.open('restarted');
    // The boundary first, as every connection sends it — see [Held.live].
    runner.latest.caughtUp();
    runner.latest.send({ kind: 'busy', status: 'requesting' }, 1);
    expect(held.doing()).toBe('requesting');
    expect(held.spoken()).toBe(true);

    runner.latest.reset();

    expect(held.doing()).toBeUndefined();
    expect(held.since()).toBeUndefined();
    expect(held.spoken()).toBe(false);
  });

  it('does not let the replayed transcript answer for the present', () => {
    // ⚠ **The defect this exists for, reported from the phone:**
    // the page said `idle` for twelve minutes over a session that was working
    // the whole time, so messages sent to it looked like messages going nowhere.
    //
    // The seed is the tail of the transcript and it ends, as almost every tail
    // does, with the `turn` that closed the *previous* piece of work. Applied as
    // if it were news, that clears `doing` and — the real damage — sets `spoken`,
    // which switches off the fallback to the runner's own flag. The CLI
    // announces a status only when it CHANGES, so a session already working when
    // this client joined says nothing further until it stops: nothing ever
    // corrects the claim.
    //
    // The runner marks the boundary per connection, once the backlog is
    // flushed. Everything before it is history and may not speak for now.
    const held = store.open('seeded');
    runner.latest.send({ kind: 'busy', status: 'requesting' }, 1);
    runner.latest.send({ kind: 'turn', cost_usd: 0, turns: 3, duration_ms: 1000 }, 2);
    runner.latest.caughtUp();

    expect(held.doing(), 'history says nothing about what is happening now').toBeUndefined();
    expect(
      held.spoken(),
      'the replay is not the stream speaking, so the summary still answers',
    ).toBe(false);

    // And the first genuinely live status takes over, as it always did.
    runner.latest.send({ kind: 'busy', status: 'tool_use' }, 4);
    expect(held.doing()).toBe('tool_use');
    expect(held.spoken()).toBe(true);
  });

  it('closes the stream when the reader jumps, so nothing is appended under the past', () => {
    // ⚠ **The reason a jump is not just another page.** Left running, the next
    // thing the session said would land under an hour-old page with nothing
    // between them and no way to tell the join from a continuation.
    const held = store.open('jumper');
    say('the newest thing', 20);

    store.goTo('jumper', 4096).subscribe();

    expect(runner.asked).toEqual([4096]);
    expect(first(runner.opened).closed, 'the live stream is left behind').toBe(true);
    expect(held.adrift()).toBe(true);
    // And nothing is claimed about the present any more: the stream that would
    // have said is closed, so a spinner left running would be from before.
    expect(held.live()).toBe(false);
    expect(held.spoken()).toBe(false);
    expect(held.doing()).toBeUndefined();
  });

  it('replaces what is on screen with the page jumped to, rather than gluing them', () => {
    // Prepending is right for the page BEFORE this one and wrong for somewhere
    // else entirely — it would invent a conversation that never happened.
    const held = store.open('jumper');
    say('the newest thing', 20);
    runner.page = {
      events: [{ kind: 'text', text: 'what was said back then' }],
      from: 3000,
    };

    store.goTo('jumper', 4096).subscribe();

    const texts = said(held.entries());
    expect(texts).toContain('what was said back then');
    expect(texts, 'the live page is gone, not above it').not.toContain('the newest thing');
    // The cursor moves to where that page began, so reading further back from a
    // jump continues from there rather than from the live end.
    expect(held.cursor()).toBe(3000);
  });

  it('throws the jumped-to page away on the way back to now', () => {
    // The stream about to arrive replays the end of the file. Keeping the middle
    // of it as well would draw the same conversation twice with a hole between.
    const held = store.open('jumper');
    say('the newest thing', 20);
    runner.page = { events: [{ kind: 'text', text: 'back then' }], from: 3000 };
    store.goTo('jumper', 4096).subscribe();

    const again = store.rejoin('jumper');

    expect(again).toBe(held);
    expect(held.adrift()).toBe(false);
    expect(held.entries()).toEqual([]);
    expect(held.cursor()).toBe(0);
    // Everything, because nothing is held to resume from.
    expect(runner.latest.after).toBe(0);
    expect(runner.latest.closed).toBe(false);
  });

  it('forgets the transcripts nobody has looked at for longest', () => {
    // A phone is not the place to hold every conversation ever opened. The one
    // being read is never a candidate — it has a stream on it.
    for (const id of ['a', 'b', 'c', 'd', 'e']) {
      store.open(id);
      say(`in ${id}`, 2);
      store.leave(id);
    }
    store.open('a');
    expect(runner.latest.after).toBe(0);
    store.leave('a');
    store.open('e');
    expect(runner.latest.after).toBe(3);
  });

  describe('a stream that drops', () => {
    // ⚠ **The browser retries on its own, about every three seconds** — measured
    // against the phone-width harness, whose mocked stream ends at once and was
    // re-requested five times in fifteen seconds. So a marker on the raw state
    // would blink at a reader whose connection is fine. What is drawn is
    // CONTINUOUS loss.
    //
    // ⚠ Fake timers are installed and removed AROUND each test rather than inside
    // it: a failed assertion skips whatever follows it, and one leaked set of fake
    // timers stalled the next three tests into their own timeouts.
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it('says nothing in the gap before the browser has retried', () => {
      // The blink this exists to prevent: three seconds of disconnection is the
      // ORDINARY case, not news.
      const held = store.open('s1');
      runner.latest.offline();
      expect(held.link.worth(), 'warned the instant the stream dropped').toBeUndefined();
      vi.advanceTimersByTime(3000);
      expect(held.link.worth(), 'warned while the browser was still retrying').toBeUndefined();
    });

    it('says so once the stream has been down long enough', () => {
      const held = store.open('s1');
      runner.latest.offline();
      vi.advanceTimersByTime(8000);
      expect(held.link.worth()).toEqual({ kind: 'stream' });
    });

    it('takes it back the moment anything arrives', () => {
      const held = store.open('s1');
      runner.latest.offline();
      vi.advanceTimersByTime(8000);
      expect(held.link.worth()).toEqual({ kind: 'stream' });
      runner.latest.send({ at: 2, kind: 'text', text: 'back' }, 1);
      expect(held.link.worth(), 'the marker outlived the reconnection').toBeUndefined();
    });

    it('starts the count again rather than carrying it, on a second drop', () => {
      const held = store.open('s1');
      runner.latest.offline();
      vi.advanceTimersByTime(5000);
      runner.latest.send({ at: 1, kind: 'text', text: 'briefly back' }, 1);
      runner.latest.offline();
      vi.advanceTimersByTime(5000);
      expect(held.link.worth(), 'the first drop’s count was still running').toBeUndefined();
      vi.advanceTimersByTime(3000);
      expect(held.link.worth()).toEqual({ kind: 'stream' });
    });

    it('does not count down for a transcript nobody is reading', () => {
      // Leaving closes the stream, so the drop that follows is this app's own
      // doing and is not news.
      const held = store.open('s1');
      runner.latest.offline();
      store.leave('s1');
      vi.advanceTimersByTime(60_000);
      expect(held.link.worth()).toBeUndefined();
    });
  });
});
