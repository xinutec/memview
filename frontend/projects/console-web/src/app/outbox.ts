import { DestroyRef, Injectable, inject, signal } from '@angular/core';
import { HttpErrorResponse } from '@angular/common/http';
import { firstValueFrom } from 'rxjs';

import { ConsoleApi } from './console-api';
import { Local } from './local';
import type { Picture } from './picture';
import type { Refusal } from './generated/Refusal';

/**
 * Messages written on this phone and not yet confirmed by the Mac: sent from here
 * until the Mac says it has them, so a message written offline goes when the
 * phone is back, and a receipt lost on the way back is retried rather than
 * guessed at.
 *
 * Each message has an id from the moment Send is pressed, and every retry carries
 * it, so the Mac answers a retry without sending it again (console/src/sent.rs).
 * What the phone cannot know after a send it never heard back from — whether the
 * message got there — it says so, and an edit or a cancel of such a message is a
 * question put to the Mac, never a pretence.
 *
 * Answers to a session's questions are never put here: one written against a
 * question may arrive after that question is gone.
 */

/** One message waiting to be confirmed. */
export interface Outgoing {
  readonly id: string;
  readonly session: string;
  readonly text: string;
  readonly pictures?: readonly Picture[];
  /** An edit of a message that may already have arrived: sent only if that one has not. */
  readonly replaces?: string;
  /** When Send was pressed, in epoch milliseconds. */
  readonly created: number;
  /**
   * Whether a send was ever attempted. Until then the message never left the phone
   * and can be changed or dropped here alone; after, it may already be there.
   */
  readonly tried: boolean;
  /** Taking back a message that may already be there, waiting to ask the Mac. */
  readonly cancelling?: boolean;
  /** Why the Mac would not take it: the session's queue stops here until decided. */
  readonly refused?: string;
}

/** What one answer from the Mac means for the message sent. */
export type Verdict =
  | { readonly kind: 'sent' }
  /** No answer worth believing: the network, or the Mac failing. Same id later. */
  | { readonly kind: 'retry' }
  /** Taken back earlier; this was a late copy of it. */
  | { readonly kind: 'cancelled' }
  | { readonly kind: 'refused'; readonly why: string };

/** The oldest message to send for `session`, or none while one there needs deciding. */
export function nextFor(outbox: readonly Outgoing[], session: string): Outgoing | undefined {
  const head = outbox.find((message) => message.session === session);
  return head?.refused === undefined ? head : undefined;
}

/** What the Mac's answer to a send means. `status` 0 is no answer at all. */
export function judged(status: number, body: unknown, now: Date = new Date()): Verdict {
  if (status >= 200 && status < 300) return { kind: 'sent' };
  if (status === 0 || status >= 500 || status === 408 || status === 429) return { kind: 'retry' };
  const refusal = refusalIn(body);
  if (status === 409 && refusal) {
    if (refusal.why === 'cancelled') return { kind: 'cancelled' };
    const when = refusal.at === undefined ? '' : ` at ${clock(refusal.at, now)}`;
    return {
      kind: 'refused',
      why:
        refusal.why === 'delivered'
          ? `the original reached the session${when}, so this edit was not sent`
          : 'the original was already handed to the session, so this edit was not sent',
    };
  }
  return {
    kind: 'refused',
    why: typeof body === 'string' && body ? body : `the Mac said ${status}`,
  };
}

/** What the Mac's answer to a cancel means: gone, already there (and when), or ask again. */
export function cancelJudged(
  status: number,
  body: unknown,
  now: Date = new Date(),
):
  | { readonly kind: 'gone' }
  | { readonly kind: 'retry' }
  | { readonly kind: 'there'; readonly note: string } {
  if (status >= 200 && status < 300) return { kind: 'gone' };
  const refusal = refusalIn(body);
  if (status === 409 && refusal?.why === 'delivered') {
    const when = refusal.at === undefined ? '' : ` at ${clock(refusal.at, now)}`;
    return { kind: 'there', note: `it had already reached the session${when}` };
  }
  if (status === 409 && refusal?.why === 'in_flight') {
    return { kind: 'there', note: 'it had already been handed to the session' };
  }
  return { kind: 'retry' };
}

/**
 * The line under a waiting message: how it stands, and for how long. `held` when
 * an earlier message of its session was refused, which stops the queue there.
 */
export function standing(message: Outgoing, now: number, held = false): string {
  if (message.refused !== undefined) return `not sent — ${message.refused}`;
  const waited = age(now - message.created);
  if (message.cancelling) return `taking back — may already have been delivered${waited}`;
  if (message.tried) return `may already have been delivered — no answer yet${waited}`;
  if (held) return `waiting until the message above is sent or discarded${waited}`;
  return `waiting to send${waited}`;
}

/** Whether a message of `outbox` waits behind a refused one of its session. */
export function heldBack(outbox: readonly Outgoing[], message: Outgoing): boolean {
  const own = outbox.filter((it) => it.session === message.session);
  const refused = own.findIndex((it) => it.refused !== undefined);
  return refused !== -1 && own.indexOf(message) > refused;
}

function age(ms: number): string {
  const minutes = Math.floor(ms / 60_000);
  if (minutes < 1) return '';
  if (minutes < 60) return ` · ${minutes} min`;
  const hours = Math.floor(minutes / 60);
  return ` · ${hours} h`;
}

function clock(at: number, now: Date): string {
  const then = new Date(at);
  const time = then.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
  return then.toDateString() === now.toDateString()
    ? time
    : `${then.toLocaleDateString(undefined, { day: 'numeric', month: 'short' })} ${time}`;
}

const WHYS: readonly Refusal['why'][] = ['cancelled', 'delivered', 'in_flight'];

/** One field of something read from outside, unchecked until its use checks it. */
function field(from: object, key: string): unknown {
  const value: unknown = Reflect.get(from, key);
  return value;
}

function textIn(from: object, key: string): string | undefined {
  const value = field(from, key);
  return typeof value === 'string' ? value : undefined;
}

/** The Mac's refusal, when `body` is one; anything else is not taken on trust. */
function refusalIn(body: unknown): Refusal | undefined {
  if (typeof body !== 'object' || body === null) return undefined;
  const why = WHYS.find((it) => it === field(body, 'why'));
  if (!why) return undefined;
  const at = field(body, 'at');
  return {
    why,
    at: typeof at === 'number' ? at : undefined,
    original: field(body, 'original') === true ? true : undefined,
  };
}

/** A picture read back from this device's storage: what a send needs of it. */
function pictureIn(kept: unknown): Picture | undefined {
  if (typeof kept !== 'object' || kept === null) return undefined;
  const data = textIn(kept, 'data');
  const mediaType = textIn(kept, 'mediaType');
  if (data === undefined || mediaType === undefined) return undefined;
  const number = (key: string) => {
    const value = field(kept, key);
    return typeof value === 'number' ? value : 0;
  };
  return {
    data,
    mediaType,
    width: number('width'),
    height: number('height'),
    bytes: number('bytes'),
    preview: textIn(kept, 'preview') ?? '',
  };
}

/** A message read back from this device's storage, when it is one. */
function outgoingIn(kept: unknown): Outgoing | undefined {
  if (typeof kept !== 'object' || kept === null) return undefined;
  const id = textIn(kept, 'id');
  const session = textIn(kept, 'session');
  const text = textIn(kept, 'text');
  const created = field(kept, 'created');
  if (!id || !session || text === undefined || typeof created !== 'number') return undefined;
  const pictures: unknown = field(kept, 'pictures');
  const read = Array.isArray(pictures)
    ? pictures.map(pictureIn).filter((it): it is Picture => it !== undefined)
    : [];
  return {
    id,
    session,
    text,
    pictures: read.length ? read : undefined,
    replaces: textIn(kept, 'replaces'),
    created,
    tried: field(kept, 'tried') === true,
    cancelling: field(kept, 'cancelling') === true ? true : undefined,
    refused: textIn(kept, 'refused'),
  };
}

/** An answer from the Mac, or none, as a status and a body. */
async function answered(sent: Promise<unknown>): Promise<{ status: number; body: unknown }> {
  try {
    return { status: 200, body: await sent };
  } catch (failed: unknown) {
    if (failed instanceof HttpErrorResponse) return { status: failed.status, body: failed.error };
    return { status: 0, body: undefined };
  }
}

const KEY = 'outbox';

/** How often a waiting message is tried again while the phone has one. */
const RETRY_MS = 15_000;

@Injectable({ providedIn: 'root' })
export class Outbox {
  private api = inject(ConsoleApi);
  private store = inject(Local);

  private readonly held = signal<readonly Outgoing[]>([]);
  /** Every message waiting, oldest first. */
  readonly all = this.held.asReadonly();
  /** Something to tell about a message that is gone: why a cancel came too late. */
  readonly notice = signal('');

  private loaded: Promise<void>;
  /** The drain under way: a second call joins it rather than starting its own. */
  private draining?: Promise<void>;
  /** Asked for during a drain, which then looks once more before it ends. */
  private again = false;
  private timer?: ReturnType<typeof setInterval>;

  constructor() {
    this.loaded = this.store.get(KEY).then((kept) => {
      // A message that cannot be read back is dropped rather than sent wrong.
      if (Array.isArray(kept)) {
        this.held.set(kept.map(outgoingIn).filter((it): it is Outgoing => it !== undefined));
      }
    });
    void this.loaded.then(() => this.drain());
    const wake = () => void this.drain();
    if (typeof window !== 'undefined') {
      window.addEventListener('online', wake);
      document.addEventListener('visibilitychange', () => {
        if (document.visibilityState === 'visible') wake();
      });
    }
    this.timer = setInterval(() => {
      if (this.held().length) wake();
    }, RETRY_MS);
    inject(DestroyRef).onDestroy(() => clearInterval(this.timer));
  }

  /** Messages waiting for one session. */
  for(session: string): readonly Outgoing[] {
    return this.held().filter((message) => message.session === session);
  }

  /** Put a message in the outbox and start sending. */
  async enqueue(session: string, text: string, pictures?: readonly Picture[]): Promise<void> {
    await this.put({
      id: crypto.randomUUID(),
      session,
      text,
      pictures: pictures?.length ? pictures : undefined,
      created: Date.now(),
      tried: false,
    });
  }

  /**
   * Take a message back. One that never left the phone is dropped here; one that
   * may already be there is asked of the Mac, which knows.
   */
  async cancel(id: string): Promise<void> {
    await this.loaded;
    const message = this.find(id);
    if (!message) return;
    if (!message.tried) return this.drop(id);
    await this.save(this.held().map((it) => (it.id === id ? { ...it, cancelling: true } : it)));
    void this.drain();
  }

  /**
   * Change a message. One that never left the phone is changed here; one that may
   * already be there becomes an edit the Mac sends only if the original has not
   * arrived, and refuses, with when, if it has.
   */
  async edit(id: string, text: string): Promise<void> {
    await this.loaded;
    const message = this.find(id);
    if (!message) return;
    const outbox = this.held();
    const replaced: Outgoing = message.tried
      ? {
          ...message,
          id: crypto.randomUUID(),
          text,
          replaces: message.id,
          tried: false,
          refused: undefined,
        }
      : { ...message, text, refused: undefined };
    await this.save(outbox.map((it) => (it.id === id ? replaced : it)));
    void this.drain();
  }

  /** Send a refused message after all, as a new one. */
  async resend(id: string): Promise<void> {
    await this.loaded;
    const message = this.find(id);
    if (!message) return;
    await this.drop(id);
    await this.enqueue(message.session, message.text, message.pictures);
  }

  /** Drop a message without sending it. */
  async discard(id: string): Promise<void> {
    await this.loaded;
    await this.drop(id);
  }

  /** The session has it: the Mac said so on the event stream. */
  confirm(id: string): void {
    if (this.find(id)) void this.drop(id);
  }

  /**
   * Send what can be sent, one message at a time per session, oldest first. One
   * drain at a time: two would send one message twice at once.
   */
  drain(): Promise<void> {
    if (this.draining) {
      this.again = true;
      return this.draining;
    }
    this.draining = this.drained().finally(() => {
      this.draining = undefined;
    });
    return this.draining;
  }

  private async drained(): Promise<void> {
    await this.loaded;
    // Known to be offline, a send cannot leave: not trying keeps it plainly
    // waiting, rather than "may already have been delivered".
    if (typeof navigator !== 'undefined' && !navigator.onLine) return;
    for (;;) {
      this.again = false;
      const sessions = [...new Set(this.held().map((message) => message.session))];
      const next = sessions
        .map((session) => nextFor(this.held(), session))
        .find((message) => message !== undefined);
      const moved =
        next !== undefined &&
        (next.cancelling ? await this.withdraw(next) : await this.deliver(next));
      if (!moved && !this.again) return;
    }
  }

  /** One send; whether the outbox moved on, so the drain carries on. */
  private async deliver(message: Outgoing): Promise<boolean> {
    // Before the request: a phone killed mid-send must remember it may have gone.
    if (!message.tried) await this.mark(message.id, { tried: true });
    const sent = message.pictures
      ? this.api.show(message.session, message.pictures, message.text, message.id, message.replaces)
      : this.api.send(message.session, message.text, message.id, message.replaces);
    const { status, body } = await answered(firstValueFrom(sent));
    const verdict = judged(status, body);
    switch (verdict.kind) {
      case 'sent':
        // Taken back while the send was on its way: too late, and said so.
        if (this.find(message.id)?.cancelling) {
          this.notice.set('Not taken back: it had already reached the session.');
        }
        await this.drop(message.id);
        return true;
      case 'cancelled':
        await this.drop(message.id);
        return true;
      case 'refused':
        await this.mark(message.id, { refused: verdict.why });
        return true;
      case 'retry':
        return false;
    }
  }

  /** One cancel; whether the outbox moved on. */
  private async withdraw(message: Outgoing): Promise<boolean> {
    const { status, body } = await answered(
      firstValueFrom(this.api.cancel(message.session, message.id)),
    );
    const verdict = cancelJudged(status, body);
    if (verdict.kind === 'retry') return false;
    if (verdict.kind === 'there') this.notice.set(`Not taken back: ${verdict.note}.`);
    await this.drop(message.id);
    return true;
  }

  private find(id: string): Outgoing | undefined {
    return this.held().find((message) => message.id === id);
  }

  private async put(message: Outgoing): Promise<void> {
    await this.loaded;
    await this.save([...this.held(), message]);
    void this.drain();
  }

  private async mark(id: string, change: Partial<Outgoing>): Promise<void> {
    await this.save(this.held().map((it) => (it.id === id ? { ...it, ...change } : it)));
  }

  private async drop(id: string): Promise<void> {
    await this.save(this.held().filter((message) => message.id !== id));
  }

  private async save(outbox: readonly Outgoing[]): Promise<void> {
    this.held.set(outbox);
    await this.store.set(KEY, outbox);
  }
}
