import { DestroyRef, Injectable, inject, signal } from '@angular/core';
import { HttpErrorResponse } from '@angular/common/http';
import { firstValueFrom } from 'rxjs';

import { ConsoleApi } from './console-api';
import { Local } from './local';
import type { Refusal } from './generated/Refusal';
import type { Picture } from './picture';

/**
 * Messages written on this phone and not yet read by their session: sent from
 * here until the Mac has them, and kept until the session reads them, so a
 * message written offline goes when the phone is back, a receipt lost on the way
 * back is retried rather than guessed at, and any message not yet read can be
 * taken back into the composer.
 *
 * Each message has an id from the moment Send is pressed, and every retry carries
 * it, so the Mac answers a retry without sending it again (console/src/sent.rs).
 * Taking back a message the Mac may have is asked of the Mac, which takes it from
 * the CLI's queue or says the session read it first — never pretended here.
 *
 * Answers to a session's questions are never put here: one written against a
 * question may arrive after that question is gone.
 */

/** One message its session has not read. */
export interface Outgoing {
  readonly id: string;
  readonly session: string;
  readonly text: string;
  readonly pictures?: readonly Picture[];
  /** When Send was pressed, in epoch milliseconds. */
  readonly created: number;
  /**
   * Whether a send was ever attempted. Until then it never left the phone; after,
   * with no answer, the Mac may have it.
   */
  readonly tried: boolean;
  /** The Mac has it: waiting for the session to read it. */
  readonly sent?: boolean;
  /** Why the Mac would not take it: its session's queue stops here until it is taken back. */
  readonly refused?: string;
}

/** What one answer from the Mac to a send means. */
export type Verdict =
  | { readonly kind: 'sent' }
  /** No answer worth believing: the network, or the Mac failing. Same id later. */
  | { readonly kind: 'retry' }
  /** Taken back earlier; this was a late copy of it. */
  | { readonly kind: 'withdrawn' }
  | { readonly kind: 'refused'; readonly why: string };

/** What the Mac's answer to taking a message back means. */
export type Taken =
  | { readonly kind: 'back' }
  | { readonly kind: 'read'; readonly note: string }
  | { readonly kind: 'unreachable' };

/** How a waiting message stands, for the line under it. */
export interface Standing {
  readonly line: string;
  readonly icon: string;
  readonly alert?: boolean;
}

/** The oldest message `session` still has to send, or none while one there needs deciding. */
export function nextFor(outbox: readonly Outgoing[], session: string): Outgoing | undefined {
  const head = outbox.find((message) => message.session === session && !message.sent);
  return head?.refused === undefined ? head : undefined;
}

/** What the Mac's answer to a send means. `status` 0 is no answer at all. */
export function judged(status: number, body: unknown): Verdict {
  if (status >= 200 && status < 300) return { kind: 'sent' };
  if (status === 0 || status >= 500 || status === 408 || status === 429) return { kind: 'retry' };
  if (status === 409 && refusalIn(body)?.why === 'withdrawn') return { kind: 'withdrawn' };
  return {
    kind: 'refused',
    why: typeof body === 'string' && body ? body : `the Mac said ${status}`,
  };
}

/** What the Mac's answer to taking a message back means. */
export function takenJudged(status: number, body: unknown, now: Date = new Date()): Taken {
  if (status >= 200 && status < 300) return { kind: 'back' };
  const refusal = refusalIn(body);
  if (status === 409 && refusal?.why === 'read') {
    const when = refusal.at === undefined ? '' : ` at ${clock(refusal.at, now)}`;
    return { kind: 'read', note: `the session read it${when}` };
  }
  return { kind: 'unreachable' };
}

/**
 * How a message stands, and for how long it has waited. `behind` when an earlier
 * message of its session was refused, which stops the queue there.
 */
export function standing(message: Outgoing, now: number, behind = false): Standing {
  if (message.refused !== undefined) {
    return { line: `not sent — ${message.refused}`, icon: 'error', alert: true };
  }
  const waited = age(now - message.created);
  if (message.sent) return { line: `waiting to be read${waited}`, icon: 'schedule' };
  if (message.tried) return { line: `sent, no answer yet${waited}`, icon: 'sync' };
  if (behind) {
    return {
      line: `waiting until the message above is taken back${waited}`,
      icon: 'schedule_send',
    };
  }
  return { line: `waiting to send${waited}`, icon: 'schedule_send' };
}

/** Whether `message` waits behind a refused message of its session. */
export function behindRefused(outbox: readonly Outgoing[], message: Outgoing): boolean {
  const own = outbox.filter((it) => it.session === message.session);
  const refused = own.findIndex((it) => it.refused !== undefined);
  return refused !== -1 && own.indexOf(message) > refused;
}

function age(ms: number): string {
  const minutes = Math.floor(ms / 60_000);
  if (minutes < 1) return '';
  if (minutes < 60) return ` · ${minutes} min`;
  return ` · ${Math.floor(minutes / 60)} h`;
}

function clock(at: number, now: Date): string {
  const then = new Date(at);
  const time = then.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
  return then.toDateString() === now.toDateString()
    ? time
    : `${then.toLocaleDateString(undefined, { day: 'numeric', month: 'short' })} ${time}`;
}

const WHYS: readonly Refusal['why'][] = ['withdrawn', 'read'];

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
  return { why, at: typeof at === 'number' ? at : undefined };
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
    // A preview's object URL dies with the page that made it; this one is whole.
    preview: `data:${mediaType};base64,${data}`,
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
    created,
    tried: field(kept, 'tried') === true,
    sent: field(kept, 'sent') === true ? true : undefined,
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

/** How often a waiting message is tried again while the phone has one to send. */
const RETRY_MS = 15_000;

@Injectable({ providedIn: 'root' })
export class Outbox {
  private api = inject(ConsoleApi);
  private store = inject(Local);

  private readonly held = signal<readonly Outgoing[]>([]);
  /** Every message not yet read, oldest first. */
  readonly all = this.held.asReadonly();

  private loaded: Promise<void>;
  /** The drain under way: a second call joins it rather than starting its own. */
  private draining?: Promise<void>;
  /** Asked for during a drain, which then looks once more before it ends. */
  private again = false;

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
    const timer = setInterval(() => {
      if (this.held().some((message) => !message.sent)) wake();
    }, RETRY_MS);
    inject(DestroyRef).onDestroy(() => clearInterval(timer));
  }

  /** Put a message in the outbox and start sending. */
  async enqueue(session: string, text: string, pictures?: readonly Picture[]): Promise<void> {
    await this.loaded;
    await this.save([
      ...this.held(),
      {
        id: crypto.randomUUID(),
        session,
        text,
        pictures: pictures?.length ? pictures : undefined,
        created: Date.now(),
        tried: false,
      },
    ]);
    void this.drain();
  }

  /** The message held under `id`, for its words. */
  find(id: string): Outgoing | undefined {
    return this.held().find((message) => message.id === id);
  }

  /**
   * Take a message back, for its words to go back into the composer: here, when
   * it never left the phone or was refused; from the Mac, which asks the CLI,
   * when the Mac may have it.
   */
  async takeBack(id: string): Promise<Taken> {
    await this.loaded;
    const message = this.find(id);
    if (!message) return { kind: 'unreachable' };
    if (!message.tried || message.refused !== undefined) {
      await this.drop(id);
      return { kind: 'back' };
    }
    return this.withdraw(message.session, id);
  }

  /**
   * Take back a message the Mac may have — this phone's, or one sent from
   * elsewhere and still waiting to be read — by asking the Mac.
   */
  async withdraw(session: string, id: string): Promise<Taken> {
    const { status, body } = await answered(firstValueFrom(this.api.withdraw(session, id)));
    const taken = takenJudged(status, body);
    if (taken.kind === 'back') await this.drop(id);
    return taken;
  }

  /** The Mac has it: the event stream said so. */
  accepted(id: string): void {
    if (this.find(id)) void this.mark(id, { tried: true, sent: true });
  }

  /** Read by its session, or taken back from another device: no longer waiting. */
  done(id: string): void {
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
    // waiting, rather than "sent, no answer yet".
    if (typeof navigator !== 'undefined' && !navigator.onLine) return;
    for (;;) {
      this.again = false;
      const sessions = [...new Set(this.held().map((message) => message.session))];
      const next = sessions
        .map((session) => nextFor(this.held(), session))
        .find((message) => message !== undefined);
      const moved = next !== undefined && (await this.deliver(next));
      if (!moved && !this.again) return;
    }
  }

  /** One send; whether the outbox moved on, so the drain carries on. */
  private async deliver(message: Outgoing): Promise<boolean> {
    // Before the request: a phone killed mid-send must remember it may have gone.
    if (!message.tried) await this.mark(message.id, { tried: true });
    const sent = message.pictures
      ? this.api.show(message.session, message.pictures, message.text, message.id)
      : this.api.send(message.session, message.text, message.id);
    const { status, body } = await answered(firstValueFrom(sent));
    const verdict = judged(status, body);
    // Taken back while this send was on its way: the outbox no longer has it.
    if (!this.find(message.id)) return true;
    switch (verdict.kind) {
      case 'sent':
        await this.mark(message.id, { sent: true });
        return true;
      case 'withdrawn':
        await this.drop(message.id);
        return true;
      case 'refused':
        await this.mark(message.id, { refused: verdict.why });
        return true;
      case 'retry':
        return false;
    }
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
