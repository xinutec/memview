import { HttpErrorResponse } from '@angular/common/http';
import { TestBed } from '@angular/core/testing';
import { type Observable, of, throwError } from 'rxjs';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { ConsoleApi } from './console-api';
import { Local } from './local';
import { Outbox, type Outgoing, cancelJudged, heldBack, judged, nextFor, standing } from './outbox';

/**
 * Messages written on the phone and sent until the Mac confirms them. The Mac's
 * side — a retry answered without a second message, an edit refused once its
 * original arrived — is tested in Rust (`console/tests/suite/outbox.rs`); this is
 * the phone's: what it keeps, what it says, and what it asks.
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

  it('stops a session at a message that needs deciding, and only that session', () => {
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

  it('says when the original of an edit got there', () => {
    const verdict = judged(409, { why: 'delivered', at: NOON.getTime(), original: true }, NOON);
    expect(verdict.kind).toBe('refused');
    expect(verdict.kind === 'refused' && verdict.why).toBe(
      `the original reached the session at ${AT_NOON}, so this edit was not sent`,
    );
  });

  it('drops a late copy of a cancelled message', () => {
    expect(judged(409, { why: 'cancelled' })).toEqual({ kind: 'cancelled' });
  });

  it('passes on what the Mac said when it refused outright', () => {
    expect(judged(404, 'no session s')).toEqual({ kind: 'refused', why: 'no session s' });
  });
});

describe('cancelJudged', () => {
  it('says a message that got there was not taken back, and when', () => {
    expect(cancelJudged(409, { why: 'delivered', at: NOON.getTime() }, NOON)).toEqual({
      kind: 'there',
      note: `it had already reached the session at ${AT_NOON}`,
    });
  });

  it('asks again when there was no answer', () => {
    expect(cancelJudged(0, undefined).kind).toBe('retry');
  });
});

describe('standing', () => {
  it('tells a message that never left the phone from one that may have arrived', () => {
    expect(standing(message(), 0)).toBe('waiting to send');
    expect(standing(message({ tried: true }), 0)).toBe(
      'may already have been delivered — no answer yet',
    );
  });

  it('says how long it has waited', () => {
    expect(standing(message(), 5 * 60_000)).toBe('waiting to send · 5 min');
    expect(standing(message(), 3 * 3_600_000)).toBe('waiting to send · 3 h');
  });

  it('says a message waits on a refused one above it', () => {
    const refused = message({ id: 'a', refused: 'the session has ended' });
    const after = message({ id: 'b' });
    const outbox = [refused, after];
    expect(heldBack(outbox, after)).toBe(true);
    expect(heldBack(outbox, refused)).toBe(false);
    expect(standing(after, 0, true)).toBe('waiting until the message above is sent or discarded');
  });

  it('says why it was not sent', () => {
    expect(standing(message({ refused: 'the session has ended' }), 0)).toBe(
      'not sent — the session has ended',
    );
  });
});

/** The Mac, answering each request in turn from a script. */
class Mac {
  readonly sends: { text: string; id?: string; replaces?: string }[] = [];
  readonly cancels: string[] = [];
  answers: (() => Observable<unknown>)[] = [];

  private next(): Observable<never> {
    const answer = this.answers.shift() ?? (() => offline());
    return answer() as Observable<never>;
  }

  send(_session: string, text: string, id?: string, replaces?: string): Observable<never> {
    this.sends.push({ text, id, replaces });
    return this.next();
  }

  show(): Observable<never> {
    return this.next();
  }

  cancel(_session: string, id: string): Observable<never> {
    this.cancels.push(id);
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

  it('keeps a message written offline, and sends it under the same id when back', async () => {
    mac.answers = [offline];
    await outbox.enqueue('s', 'hello');
    await outbox.drain();
    expect(outbox.all()).toHaveLength(1);
    expect(outbox.all()[0]?.tried).toBe(true);

    mac.answers = [ok];
    await outbox.drain();
    expect(outbox.all()).toHaveLength(0);
    const [first, second] = mac.sends;
    expect(second?.id).toBe(first?.id);
  });

  it('marks a message tried before it is sent, so a phone killed mid-send remembers', async () => {
    let seen: Outgoing | undefined;
    mac.answers = [
      () => {
        seen = outbox.all()[0];
        return offline();
      },
    ];
    await outbox.enqueue('s', 'hello');
    await outbox.drain();
    expect(seen?.tried).toBe(true);
  });

  it('changes a message that never left the phone in place', async () => {
    // Offline, nothing is tried, so the message is the phone's alone to change.
    vi.spyOn(navigator, 'onLine', 'get').mockReturnValue(false);
    await outbox.enqueue('s', 'hello');
    const id = outbox.all()[0]?.id ?? '';
    await outbox.edit(id, 'hello, edited');
    expect(outbox.all()[0]).toMatchObject({ id, text: 'hello, edited', tried: false });
    expect(mac.sends).toHaveLength(0);
  });

  it('sends an edit of a message that may have arrived as a replacement of it', async () => {
    // Sent, no answer, then edited: the Mac decides, since only it knows.
    mac.answers = [offline];
    await outbox.enqueue('s', 'hello');
    await outbox.drain();
    const original = outbox.all()[0]?.id ?? '';

    mac.answers = [refused({ why: 'delivered', at: NOON.getTime(), original: true })];
    await outbox.edit(original, 'hello, edited');
    await outbox.drain();
    expect(mac.sends.at(-1)).toMatchObject({ text: 'hello, edited', replaces: original });
    expect(mac.sends.at(-1)?.id).not.toBe(original);
    const left = outbox.all()[0];
    expect(left?.refused).toMatch(/^the original reached the session/);
  });

  it('drops a message that never left the phone when it is taken back', async () => {
    vi.spyOn(navigator, 'onLine', 'get').mockReturnValue(false);
    await outbox.enqueue('s', 'hello');
    await outbox.cancel(outbox.all()[0]?.id ?? '');
    expect(outbox.all()).toHaveLength(0);
    expect(mac.cancels).toHaveLength(0);
  });

  it('asks the Mac to take back a message that may have arrived, and says when it had', async () => {
    mac.answers = [offline];
    await outbox.enqueue('s', 'hello');
    await outbox.drain();
    const id = outbox.all()[0]?.id ?? '';

    mac.answers = [refused({ why: 'delivered', at: NOON.getTime() })];
    await outbox.cancel(id);
    await outbox.drain();
    expect(mac.cancels).toEqual([id]);
    expect(outbox.all()).toHaveLength(0);
    expect(outbox.notice()).toMatch(/^Not taken back: it had already reached the session/);
  });

  it('stops waiting for a message the event stream says arrived', async () => {
    mac.answers = [offline];
    await outbox.enqueue('s', 'hello');
    await outbox.drain();
    outbox.confirm(outbox.all()[0]?.id ?? '');
    await Promise.resolve();
    expect(outbox.all()).toHaveLength(0);
  });
});
