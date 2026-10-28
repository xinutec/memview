import { HttpErrorResponse } from '@angular/common/http';
import { TestBed } from '@angular/core/testing';
import { type Observable, of, throwError } from 'rxjs';
import type { Picture } from './picture';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { ConsoleApi } from './console-api';
import { Drafts } from './drafts';
import { Local } from './local';
import type { Waiting } from './models';
import { SessionStore } from './session-store';
import {
  Outbox,
  type Outgoing,
  behindRefused,
  judged,
  nextFor,
  standing,
  takenJudged,
} from './outbox';

/**
 * Messages written on the phone until the Mac has them. The Mac's side — a retry
 * answered without a second message, a message given back once — is tested in
 * Rust (`console/tests/suite/outbox.rs`); this is the phone's: what it keeps,
 * what it says, and what it asks.
 */

const NOON = new Date(2026, 9, 27, 12, 0);
/** Noon as the phone writes a time: in its own locale, as the rest of the app does. */
const AT_NOON = NOON.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });

function message(change: Partial<Outgoing> = {}): Outgoing {
  return { id: 'm-1', session: 's', text: 'hi', created: 0, tried: false, ...change };
}

describe('nextFor', () => {
  it('sends the oldest of a session first', () => {
    const outbox = [message({ id: 'a' }), message({ id: 'b' })];
    expect(nextFor(outbox, 's')?.id).toBe('a');
  });

  it('stops a session at a refused message, and only that session', () => {
    // The next one was probably written assuming the refused one arrived.
    const outbox = [
      message({ id: 'a', refused: 'the session has ended' }),
      message({ id: 'b' }),
      message({ id: 'c', session: 'other' }),
    ];
    expect(nextFor(outbox, 's')).toBeUndefined();
    expect(nextFor(outbox, 'other')?.id).toBe('c');
  });
});

describe('judged', () => {
  it('takes a 200 as sent', () => {
    expect(judged(200, {})).toEqual({ kind: 'sent' });
  });

  it('retries when there was no answer worth believing', () => {
    for (const status of [0, 502, 503, 504]) expect(judged(status, undefined).kind).toBe('retry');
  });

  it('drops a send that was taken back while it was on its way', () => {
    expect(judged(409, { why: 'withdrawn' })).toEqual({ kind: 'withdrawn' });
  });

  it('passes on what the Mac said when it refused outright', () => {
    expect(judged(404, 'no session s')).toEqual({ kind: 'refused', why: 'no session s' });
  });
});

describe('takenJudged', () => {
  it('takes whether the Mac put the words in the draft, and its pictures', () => {
    expect(takenJudged(200, { drafted: true, pictures: ['p.png'] })).toMatchObject({
      kind: 'back',
      drafted: true,
      pictures: ['p.png'],
    });
  });

  it('says when the session read it first', () => {
    expect(takenJudged(409, { why: 'read', at: NOON.getTime() }, NOON)).toEqual({
      kind: 'refused',
      why: `the session read it at ${AT_NOON}`,
    });
  });

  it('does not pretend when there was no answer', () => {
    expect(takenJudged(0, undefined)).toEqual({ kind: 'unreachable' });
  });
});

describe('standing', () => {
  it('tells a message that never left the phone from one that may have arrived', () => {
    expect(standing(message(), 0).line).toBe('waiting to send');
    expect(standing(message({ tried: true }), 0).line).toBe('sent, no answer yet');
  });

  it('says how long it has waited', () => {
    expect(standing(message(), 5 * 60_000).line).toBe('waiting to send · 5 min');
    expect(standing(message(), 3 * 3_600_000).line).toBe('waiting to send · 3 h');
  });

  it('says a message waits behind a refused one', () => {
    const refused = message({ id: 'a', refused: 'the session has ended' });
    const after = message({ id: 'b' });
    expect(behindRefused([refused, after], after)).toBe(true);
    expect(standing(after, 0, true).line).toBe('waiting until the message above is taken back');
  });

  it('says why it was not sent, as an alert', () => {
    expect(standing(message({ refused: 'the session has ended' }), 0)).toMatchObject({
      line: 'not sent — the session has ended',
      alert: true,
    });
  });
});

/** The Mac, answering each request in turn from a script. */
class Mac {
  readonly sends: { text: string; id?: string }[] = [];
  readonly withdrawals: string[] = [];
  answers: (() => Observable<unknown>)[] = [];

  private next(): Observable<never> {
    const answer = this.answers.shift() ?? (() => offline());
    return answer() as Observable<never>;
  }

  send(_session: string, text: string, id?: string): Observable<never> {
    this.sends.push({ text, id });
    return this.next();
  }

  show(): Observable<never> {
    return this.next();
  }

  withdraw(_session: string, id: string): Observable<never> {
    this.withdrawals.push(id);
    return this.next();
  }
}

const ok = () => of({ id: 's' });
const offline = () => throwError(() => new HttpErrorResponse({ status: 0 }));
const refused = (body: unknown) => () =>
  throwError(() => new HttpErrorResponse({ status: 409, error: body }));

class Memory {
  private kept = new Map<string, unknown>();
  get(key: string): Promise<unknown> {
    return Promise.resolve(this.kept.get(key));
  }
  set(key: string, value: unknown): Promise<void> {
    this.kept.set(key, value);
    return Promise.resolve();
  }
  delete(key: string): Promise<void> {
    this.kept.delete(key);
    return Promise.resolve();
  }
}

/** The conversations, as far as the outbox tells them of sends the Mac accepted. */
class Conversations {
  readonly told: { id: string; taken: number; message: Waiting }[] = [];
  accepted(id: string, taken: number, message: Waiting): void {
    this.told.push({ id, taken, message });
  }
}

/** A conversation's draft, as the box would show it. */
class Box {
  words = new Map<string, string>();
  synced = 0;
  flushed = 0;
  sync(): void {
    this.synced += 1;
  }
  flush(): Promise<void> {
    this.flushed += 1;
    return Promise.resolve();
  }
  held = new Map<string, readonly Picture[]>();
  read(id: string): Promise<string> {
    return Promise.resolve(this.words.get(id) ?? '');
  }
  write(id: string, text: string): Promise<void> {
    this.words.set(id, text);
    return Promise.resolve();
  }
  hold(id: string, pictures: readonly Picture[]): Promise<void> {
    this.held.set(id, pictures);
    return Promise.resolve();
  }
  pictures$(id: string): Observable<readonly Picture[]> {
    return of(this.held.get(id) ?? []);
  }
}

describe('Outbox', () => {
  let mac: Mac;
  let box: Box;
  let conversations: Conversations;
  let outbox: Outbox;

  beforeEach(() => {
    mac = new Mac();
    box = new Box();
    conversations = new Conversations();
    TestBed.configureTestingModule({
      providers: [
        { provide: ConsoleApi, useValue: mac },
        { provide: Local, useValue: new Memory() },
        { provide: Drafts, useValue: box },
        { provide: SessionStore, useValue: conversations },
      ],
    });
    outbox = TestBed.inject(Outbox);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  const first = () => outbox.all()[0];

  it('keeps a message written offline, and sends it under the same id when back', async () => {
    mac.answers = [offline];
    await outbox.enqueue('s', 'hello');
    await outbox.drain();
    expect(first()?.tried).toBe(true);

    mac.answers = [ok];
    await outbox.drain();
    const [one, two] = mac.sends;
    expect(two?.id).toBe(one?.id);
    // The Mac has it now, and says where it stands; the phone keeps nothing.
    expect(outbox.all()).toHaveLength(0);
  });

  it('marks a message tried before it is sent, so a phone killed mid-send remembers', async () => {
    let seen: Outgoing | undefined;
    mac.answers = [
      () => {
        seen = first();
        return offline();
      },
    ];
    await outbox.enqueue('s', 'hello');
    await outbox.drain();
    expect(seen?.tried).toBe(true);
  });

  it('hands a send the Mac accepted to its conversation, as listed and when', async () => {
    // Dropped from here at once: the conversation shows it from this answer until
    // its stream lists as new, so it is on screen throughout.
    const listed: Waiting = { id: 'x', text: 'hello', at: 0, stands: 'unread' };
    mac.answers = [() => of({ taken: 42, message: listed })];
    await outbox.enqueue('s', 'hello');
    await outbox.drain();
    expect(conversations.told).toEqual([{ id: 's', taken: 42, message: listed }]);
    expect(outbox.all()).toHaveLength(0);
  });

  it('takes back a message that never left the phone without asking anyone', async () => {
    vi.spyOn(navigator, 'onLine', 'get').mockReturnValue(false);
    await outbox.enqueue('s', 'hello');
    expect((await outbox.takeBack('s', first()?.id ?? '')).kind).toBe('back');
    expect(box.words.get('s')).toBe('hello');
    expect(mac.withdrawals).toHaveLength(0);
  });

  it('asks the Mac about a message it may have, and gives its own words back', async () => {
    // Sent, no answer: the Mac may have it, so only the Mac can say; never having
    // had it, it remembers it as taken back, so the send still on its way is refused.
    mac.answers = [offline];
    await outbox.enqueue('s', 'hello');
    await outbox.drain();
    const id = first()?.id ?? '';
    mac.answers = [() => of({ drafted: false, pictures: [], session: { id: 's' } })];
    expect((await outbox.takeBack('s', id)).kind).toBe('back');
    expect(box.words.get('s'), 'its own words: the Mac never had them').toBe('hello');
    expect(mac.withdrawals).toEqual([id]);
    expect(outbox.all()).toHaveLength(0);
  });

  it('does not give back a message the session read', async () => {
    mac.answers = [offline];
    await outbox.enqueue('s', 'hello');
    await outbox.drain();
    mac.answers = [refused({ why: 'read', at: NOON.getTime() })];
    const taken = await outbox.takeBack('s', first()?.id ?? '');
    expect(taken.kind).toBe('refused');
    expect(outbox.all()).toHaveLength(0);
    expect(box.words.get('s'), 'read: nothing comes back').toBeUndefined();
  });

  it('does not pretend to take back a message it cannot ask about', async () => {
    mac.answers = [offline];
    await outbox.enqueue('s', 'hello');
    await outbox.drain();
    mac.answers = [offline];
    expect(await outbox.takeBack('s', first()?.id ?? '')).toEqual({ kind: 'unreachable' });
    expect(outbox.all()).toHaveLength(1);
  });

  it('takes back a refused message here, freeing its session', async () => {
    mac.answers = [
      () => throwError(() => new HttpErrorResponse({ status: 404, error: 'no session s' })),
    ];
    await outbox.enqueue('s', 'hello');
    await outbox.drain();
    expect(first()?.refused).toBe('no session s');
    expect((await outbox.takeBack('s', first()?.id ?? '')).kind).toBe('back');
    expect(mac.withdrawals).toHaveLength(0);
  });

  it('puts the words before what is already being written', async () => {
    vi.spyOn(navigator, 'onLine', 'get').mockReturnValue(false);
    await outbox.enqueue('s', 'rename the branch');
    box.words.set('s', 'and then');
    await outbox.takeBack('s', first()?.id ?? '');
    expect(box.words.get('s')).toBe('rename the branch\n\nand then');
  });

  it('leaves the words of a message the Mac had to the draft the Mac wrote', async () => {
    // The phone's own copy too: its send arrived, the answer did not. Written
    // here as well, the words would be in the box twice.
    mac.answers = [offline];
    await outbox.enqueue('s', 'hello');
    await outbox.drain();
    mac.answers = [() => of({ drafted: true, pictures: [], session: { id: 's' } })];
    expect((await outbox.takeBack('s', first()?.id ?? '')).kind).toBe('back');
    expect(box.words.get('s'), 'the Mac has put them in the draft').toBeUndefined();
    expect(box.flushed, 'what was being written went first').toBe(1);
    expect(box.synced, 'and the draft is fetched at once').toBe(1);
    expect(outbox.all()).toHaveLength(0);
  });
});
