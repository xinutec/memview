import { DestroyRef, Injectable, inject, signal } from '@angular/core';
import { HttpErrorResponse } from '@angular/common/http';
import { firstValueFrom } from 'rxjs';

import { ConsoleApi } from './console-api';
import { Drafts } from './drafts';
import { Local } from './local';
import type { Accepted } from './generated/Accepted';
import type { Refusal } from './generated/Refusal';
import { MOST, type Picture } from './picture';
import { SessionStore } from './session-store';

/**
 * Messages written on this phone that the Mac does not have yet: sent from here
 * until it does, so a message written offline goes when the phone is back, and a
 * receipt lost on the way back is retried under the same id, which the Mac
 * answers without sending it again.
 *
 * Once the Mac has a message, this phone keeps nothing of it: the conversation's
 * stream lists where it stands (see `unseen`), and taking it back is asked of the
 * Mac, which puts its words in the draft once — see console/src/session/messages.rs.
 *
 * Answers to a session's questions are never put here: one written against a
 * question may arrive after that question is gone.
 */

/** One message the Mac does not have yet. */
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
  /** Why the Mac would not take it: its session's queue stops here until it is taken back. */
  readonly refused?: string;
}

/** What one answer from the Mac to a send means. */
export type Verdict =
  | { readonly kind: 'sent' }
  /** No answer worth believing: the network, or the Mac failing. Same id later. */
  | { readonly kind: 'retry' }
  /** Taken back while this send was on its way. */
  | { readonly kind: 'withdrawn' }
  | { readonly kind: 'refused'; readonly why: string };

/** What taking a message back came to. */
export type Taken =
  /**
   * Back. `drafted` when the Mac had it and put its words in the draft itself;
   * its pictures' names on the Mac.
   */
  | {
      readonly kind: 'back';
      readonly drafted: boolean;
      readonly pictures: readonly string[];
    }
  /** Not back, and settled: read, or refused. Why, for the reader. */
  | { readonly kind: 'refused'; readonly why: string }
  /** No answer worth believing: the Mac may have it, so nothing is pretended. */
  | { readonly kind: 'unreachable' };

/** How a waiting message stands, for the line under it. */
export interface Standing {
  readonly line: string;
  readonly icon: string;
  readonly alert?: boolean;
}

/** The oldest message `session` still has to send, or none while one there needs deciding. */
export function nextFor(outbox: readonly Outgoing[], session: string): Outgoing | undefined {
  const head = outbox.find((message) => message.session === session);
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
  if (status >= 200 && status < 300) {
    const returned = returnedIn(body);
    return { kind: 'back', drafted: returned.drafted, pictures: returned.pictures };
  }
  const refusal = refusalIn(body);
  if (status === 409 && refusal?.why === 'read') {
    const when = refusal.at === undefined ? '' : ` at ${clock(refusal.at, now)}`;
    return { kind: 'refused', why: `the session read it${when}` };
  }
  if (status >= 400 && status < 500 && typeof body === 'string' && body) {
    return { kind: 'refused', why: body };
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

/** How long something has waited, for the end of its line. */
export function age(ms: number): string {
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

/** A message the Mac gave back, read field by field. */
function returnedIn(body: unknown): { drafted: boolean; pictures: string[] } {
  if (typeof body !== 'object' || body === null) return { drafted: false, pictures: [] };
  const pictures = field(body, 'pictures');
  return {
    drafted: field(body, 'drafted') === true,
    pictures: Array.isArray(pictures)
      ? pictures.filter((it): it is string => typeof it === 'string')
      : [],
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
  private drafts = inject(Drafts);
  private sessions = inject(SessionStore);

  private readonly held = signal<readonly Outgoing[]>([]);
  /** Every message the Mac does not have yet, oldest first. */
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
      if (this.held().length) wake();
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
   * Take a message back into its conversation's draft: here, when it never left
   * the phone or was refused; else from the Mac, which puts the words of one it
   * had into the draft itself, once however often it is asked — so an answer
   * lost on the way loses nothing. Never having had it, the Mac remembers it as
   * taken back, so a send still on its way is refused, and this phone gives
   * back its own words.
   *
   * The draft, not a page's box: on a slow link the answer can come after the
   * page that asked is gone.
   */
  async takeBack(session: string, id: string): Promise<Taken> {
    await this.loaded;
    const mine = this.find(id);
    const taken =
      mine && (!mine.tried || mine.refused !== undefined)
        ? { kind: 'back' as const, drafted: false, pictures: [] }
        : await this.withdraw(session, id);
    if (taken.kind === 'unreachable') return taken;
    if (taken.kind === 'back') {
      // This phone's own pictures when it has them; else the Mac's.
      const pictures = mine?.pictures ?? (await this.fetched(session, taken.pictures));
      await this.giveBack(session, taken.drafted ? '' : (mine?.text ?? ''), pictures);
      if (taken.drafted) this.drafts.sync();
    }
    // Back, read, or taken back elsewhere: either way no longer this phone's to send.
    if (mine) await this.drop(id);
    return taken;
  }

  /**
   * Take back a message the Mac may have. What is being written goes first: the
   * Mac puts the words before the draft it holds, and one it has not been sent
   * yet would run into them.
   */
  private async withdraw(session: string, id: string): Promise<Taken> {
    await this.drafts.flush();
    const { status, body } = await answered(firstValueFrom(this.api.withdraw(session, id)));
    return takenJudged(status, body);
  }

  /** Words and pictures into a conversation's box, before what is already there. */
  private async giveBack(
    session: string,
    text: string,
    pictures: readonly Picture[],
  ): Promise<void> {
    if (text) {
      const there = (await this.drafts.read(session)).trim();
      await this.drafts.write(session, there ? `${text}\n\n${there}` : text);
    }
    if (pictures.length) {
      const held = await firstValueFrom(this.drafts.pictures$(session));
      await this.drafts.hold(session, [...pictures, ...held].slice(0, MOST));
    }
  }

  /** Pictures the Mac kept, fetched back to go into the box again. */
  private async fetched(session: string, names: readonly string[]): Promise<Picture[]> {
    const back: Picture[] = [];
    for (const name of names) {
      try {
        const blob = await (await fetch(this.api.pictureAt(session, name))).blob();
        const data = await new Promise<string>((done, fail) => {
          const reader = new FileReader();
          reader.onload = () =>
            done(typeof reader.result === 'string' ? (reader.result.split(',')[1] ?? '') : '');
          reader.onerror = () => fail(reader.error ?? new Error('unreadable'));
          reader.readAsDataURL(blob);
        });
        const preview = `data:${blob.type};base64,${data}`;
        back.push({ data, mediaType: blob.type, width: 0, height: 0, bytes: blob.size, preview });
      } catch {
        // One the Mac no longer has stays out; the words still come back.
      }
    }
    return back;
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
    let accepted: Accepted | undefined;
    const { status, body } = await answered(
      firstValueFrom(sent).then((answer) => (accepted = answer)),
    );
    const verdict = judged(status, body);
    // Taken back while this send was on its way: the outbox no longer has it.
    if (!this.find(message.id)) return true;
    switch (verdict.kind) {
      case 'sent':
        // The conversation shows it from this answer until its stream lists as new.
        if (accepted?.message) {
          this.sessions.accepted(message.session, accepted.taken, accepted.message);
        }
        await this.drop(message.id);
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
