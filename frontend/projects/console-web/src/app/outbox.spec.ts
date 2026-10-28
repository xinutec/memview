import { HttpErrorResponse } from '@angular/common/http';
import { TestBed } from '@angular/core/testing';
import { type Observable, of, throwError } from 'rxjs';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { ConsoleApi } from './console-api';
import { Local } from './local';
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
 * Messages written on the phone and kept until their session reads them. The
 * Mac's side — a retry answered without a second message, a withdrawal asked of
 * the CLI — is tested in Rust (`console/tests/suite/outbox.rs`); this is the
 * phone's: what it keeps, what it says, and what it asks.
 */

const NOON = new Date(2026, 9, 27, 12, 0);
/** Noon as the phone writes a time: in its own locale, as the rest of the app does. */
const AT_NOON = NOON.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });

function message(change: Partial<Outgoing> = {}): Outgoing {
  return { id: 'm-1', session: 's', text: 'hi', created: 0, tried: false, ...change };
}

describe('nextFor', () => {
  it('sends the oldest of a session first, past what the Mac already has', () => {
    const outbox = [message({ id: 'a', tried: true, sent: true }), message({ id: 'b' })];
    expect(nextFor(outbox, 's')?.id).toBe('b');
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

  it('drops a late copy of a message taken back', () => {
    expect(judged(409, { why: 'withdrawn' })).toEqual({ kind: 'withdrawn' });
  });

  it('passes on what the Mac said when it refused outright', () => {
    expect(judged(404, 'no session s')).toEqual({ kind: 'refused', why: 'no session s' });
  });
});

describe('takenJudged', () => {
  it('takes a 200 as back', () => {
    expect(takenJudged(200, {})).toEqual({ kind: 'back' });
  });

  it('says when the session read it first', () => {
    expect(takenJudged(409, { why: 'read', at: NOON.getTime() }, NOON)).toEqual({
      kind: 'read',
      note: `the session read it at ${AT_NOON}`,
    });
  });

  it('passes on what the Mac said when it could not settle it', () => {
    expect(takenJudged(409, 'the session did not answer in time')).toEqual({
      kind: 'unsettled',
      note: 'the session did not answer in time',
    });
  });

  it('does not pretend when there was no answer', () => {
    expect(takenJudged(0, undefined).kind).toBe('unreachable');
  });
});

describe('standing', () => {
  it('tells each step from the next', () => {
    expect(standing(message(), 0).line).toBe('waiting to send');
    expect(standing(message({ tried: true }), 0).line).toBe('sent, no answer yet');
    expect(standing(message({ tried: true, sent: true }), 0).line).toBe('waiting to be read');
  });

  it('says how long it has waited', () => {
    expect(standing(message(), 5 * 60_000).line).toBe('waiting to send · 5 min');
    expect(standing(message(), 3 * 3_600_000).line).toBe('waiting to send · 3 h');
  });

  it('says a message waits behind a refused one', () => {
    const refused = message({ id: 'a', refused: 'the session has ended' });
    const after = message({ id: 'b' });
    expect(behindRefused([refused, after], after)).toBe(true);
    expect(behindRefused([refused, after], refused)).toBe(false);
    expect(standing(after, 0, true).line).toBe('waiting until the message above is taken back');
  });

  it('says why it was not sent, as an alert', () => {
    const refused = standing(message({ refused: 'the session has ended' }), 0);
    expect(refused).toMatchObject({ line: 'not sent — the session has ended', alert: true });
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

const ok = () => of({});
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

describe('Outbox', () => {
  let mac: Mac;
  let outbox: Outbox;

  beforeEach(() => {
    mac = new Mac();
    TestBed.configureTestingModule({
      providers: [
        { provide: ConsoleApi, useValue: mac },
        { provide: Local, useValue: new Memory() },
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
    // The Mac has it; it waits to be read, and can still be taken back.
    expect(first()).toMatchObject({ sent: true });
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

  it('takes back a message that never left the phone without asking anyone', async () => {
    vi.spyOn(navigator, 'onLine', 'get').mockReturnValue(false);
    await outbox.enqueue('s', 'hello');
    expect(await outbox.takeBack(first()?.id ?? '')).toEqual({ kind: 'back' });
    expect(outbox.all()).toHaveLength(0);
    expect(mac.withdrawals).toHaveLength(0);
  });

  it('asks the Mac to take back a message it may have', async () => {
    mac.answers = [ok];
    await outbox.enqueue('s', 'hello');
    await outbox.drain();
    const id = first()?.id ?? '';

    mac.answers = [ok];
    expect(await outbox.takeBack(id)).toEqual({ kind: 'back' });
    expect(mac.withdrawals).toEqual([id]);
    expect(outbox.all()).toHaveLength(0);
  });

  it('keeps a message the session read first, and says when', async () => {
    mac.answers = [ok];
    await outbox.enqueue('s', 'hello');
    await outbox.drain();
    const id = first()?.id ?? '';

    mac.answers = [refused({ why: 'read', at: NOON.getTime() })];
    const taken = await outbox.takeBack(id);
    expect(taken.kind).toBe('read');
    // Its echo takes it out of the outbox, into the conversation.
    outbox.done(id);
    await Promise.resolve();
    expect(outbox.all()).toHaveLength(0);
  });

  it('does not pretend to take back a message it cannot ask about', async () => {
    mac.answers = [offline];
    await outbox.enqueue('s', 'hello');
    await outbox.drain();
    mac.answers = [offline];
    expect(await outbox.takeBack(first()?.id ?? '')).toEqual({ kind: 'unreachable' });
    expect(outbox.all()).toHaveLength(1);
  });

  it('takes back a refused message here, freeing its session', async () => {
    mac.answers = [
      () => throwError(() => new HttpErrorResponse({ status: 404, error: 'no session s' })),
    ];
    await outbox.enqueue('s', 'hello');
    await outbox.drain();
    expect(first()?.refused).toBe('no session s');
    expect(await outbox.takeBack(first()?.id ?? '')).toEqual({ kind: 'back' });
    expect(mac.withdrawals).toHaveLength(0);
  });

  it('gives back the words of a message its session ended without reading', async () => {
    mac.answers = [ok];
    await outbox.enqueue('s', 'hello');
    await outbox.drain();
    const id = first()?.id ?? '';
    outbox.dropped(id);
    await Promise.resolve();
    expect(first()?.refused).toBe('the session ended before reading it');
    expect(await outbox.takeBack(id)).toEqual({ kind: 'back' });
    expect(mac.withdrawals).toHaveLength(0);
  });

  it('marks a message sent when the event stream says the Mac has it', async () => {
    mac.answers = [offline];
    await outbox.enqueue('s', 'hello');
    await outbox.drain();
    outbox.accepted(first()?.id ?? '');
    await Promise.resolve();
    expect(first()?.sent).toBe(true);
  });
});
