import { TestBed } from '@angular/core/testing';
import { beforeEach, describe, expect, it } from 'vitest';

import { ConsoleApi } from './console-api';
import { SessionEvent } from './models';
import { SessionStore } from './session-store';

/** One stream the store opened, and the handles to drive it from the test. */
interface Opened {
  readonly id: string;
  /** What the store said it already held. Zero means "nothing, send everything". */
  readonly after: number;
  readonly send: (event: SessionEvent, seq: number) => void;
  readonly reset: () => void;
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

  follow(
    id: string,
    after: number,
    onEvent: (event: SessionEvent, seq: number) => void,
    onReset: () => void,
  ): () => void {
    const stream: Opened = { id, after, send: onEvent, reset: onReset, closed: false };
    this.opened.push(stream);
    return () => (stream.closed = true);
  }

  /** The stream opened most recently, which is the one under test. */
  get latest(): Opened {
    return this.opened[this.opened.length - 1];
  }
}

describe('SessionStore', () => {
  let runner: Runner;
  let store: SessionStore;

  beforeEach(() => {
    runner = new Runner();
    TestBed.configureTestingModule({
      providers: [{ provide: ConsoleApi, useValue: runner }],
    });
    store = TestBed.inject(SessionStore);
  });

  /** Say something, as the runner would, and number it. */
  function say(text: string, seq: number): void {
    runner.latest.send({ kind: 'text', text }, seq);
    runner.latest.send({ kind: 'turn', cost_usd: 0, turns: 1 }, seq + 1);
  }

  it('resumes a session it is re-entered rather than reading it again', () => {
    // The whole reason this store exists. Held in the view, the transcript died
    // with it, so going back to the list and in again threw away every page
    // somebody had scrolled back to load — silently, and looking like a reload.
    const held = store.open('one');
    expect(runner.latest.after).toBe(0);
    say('an answer', 10);
    expect(held.entries().map((e) => e.text)).toContain('an answer');

    store.leave('one');
    expect(runner.opened[0].closed).toBe(true);

    const again = store.open('one');
    expect(again).toBe(held);
    expect(again.entries().map((e) => e.text)).toContain('an answer');
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
    expect(runner.opened[0].closed).toBe(true);
    expect(runner.opened[1].closed).toBe(false);
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
});

describe('SessionStore background tasks', () => {
  let runner: Runner;
  let store: SessionStore;

  beforeEach(() => {
    runner = new Runner();
    TestBed.configureTestingModule({
      providers: [{ provide: ConsoleApi, useValue: runner }],
    });
    store = TestBed.inject(SessionStore);
  });

  it('counts a backgrounded call until its notification arrives', () => {
    // The gap this fills: a gate running for four minutes looked exactly like
    // nothing happening, because the tool call that starts one returns at once.
    const held = store.open('one');
    runner.latest.send(
      { kind: 'tool', id: 'toolu_1', name: 'Bash', input: { run_in_background: true } },
      1,
    );
    expect(held.background()).toEqual(['toolu_1']);

    runner.latest.send({ kind: 'background', tool: 'toolu_1', status: 'completed' }, 2);
    expect(held.background()).toEqual([]);
  });

  it('does not count an ordinary tool call', () => {
    const held = store.open('two');
    runner.latest.send({ kind: 'tool', id: 'toolu_2', name: 'Bash', input: { command: 'ls' } }, 1);
    expect(held.background()).toEqual([]);
  });

  it('ignores a notification for a call it never saw start', () => {
    // Re-entry seeds from the transcript, which may begin after the call that
    // started the task. Subtracting from nothing must not go negative or throw.
    const held = store.open('three');
    runner.latest.send({ kind: 'background', tool: 'toolu_gone', status: 'completed' }, 1);
    expect(held.background()).toEqual([]);
  });
});

describe('SessionStore background tasks across a restart', () => {
  it('forgets tasks that belonged to the process before this one', () => {
    // They died with the runner and will never report, so counting them for ever
    // is worse than not counting them at all — the number stops meaning anything.
    const runner = new Runner();
    TestBed.configureTestingModule({ providers: [{ provide: ConsoleApi, useValue: runner }] });
    const store = TestBed.inject(SessionStore);
    const held = store.open('restarted');

    runner.latest.send(
      { kind: 'tool', id: 'toolu_old', name: 'Bash', input: { run_in_background: true } },
      1,
    );
    expect(held.background()).toEqual(['toolu_old']);

    runner.latest.send({ kind: 'started', model: 'claude-opus-5' }, 2);
    expect(held.background()).toEqual([]);
  });

  it('forgets tasks the replayed transcript describes', () => {
    // The restart above is only half of it: a client opening a session it was
    // not watching is sent the seed first, and the process it belongs to started
    // before anyone connected — so `started` is not in the stream to reset on.
    // Without this the count is whatever the last page of the transcript happens
    // to mention, which on a resumed session is entirely dead work.
    const runner = new Runner();
    TestBed.configureTestingModule({ providers: [{ provide: ConsoleApi, useValue: runner }] });
    const store = TestBed.inject(SessionStore);
    const held = store.open('resumed');

    runner.latest.send(
      { kind: 'tool', id: 'toolu_yesterday', name: 'Bash', input: { run_in_background: true } },
      1,
    );
    runner.latest.send({ kind: 'joined', earlier: 1, from: 4096 }, 2);
    expect(held.background()).toEqual([]);

    // And the live stream after it still counts, which is the whole point of
    // resetting there rather than refusing to count replayed calls for ever.
    runner.latest.send(
      { kind: 'tool', id: 'toolu_now', name: 'Bash', input: { run_in_background: true } },
      3,
    );
    expect(held.background()).toEqual(['toolu_now']);
  });
});
