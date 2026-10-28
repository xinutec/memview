import { Injectable, inject } from '@angular/core';

import { Local } from './local';
import type { Entry, Waiting } from './models';
import type { Listed } from './session-store';

/**
 * The last transcript this phone saw: shown the moment a session opens, until
 * the conversation itself has arrived, and the only one there is when the Mac
 * cannot be reached. Read only: what is written meanwhile waits in [[Outbox]],
 * which says how long each message has waited, since one delivered late may not
 * be what was meant.
 *
 * Local, so replication cannot carry it. No service worker: the console sits
 * behind a client certificate, where ngsw's `navigationUrls` and auth are known
 * trouble.
 */
@Injectable({ providedIn: 'root' })
export class Kept {
  private store = inject(Local);

  /**
   * How much of a conversation is kept, in entries — the end of it. Reading
   * further back needs the runner anyway.
   */
  private static readonly ENTRIES = 200;

  /** How often one session's copy is rewritten, in milliseconds. */
  private static readonly EVERY = 5_000;

  private lastWrote = new Map<string, number>();

  /**
   * What was kept for this session, or nothing. Checked field by field, not cast:
   * storage outlives every deploy that touched this phone.
   */
  async entries(id: string): Promise<Entry[]> {
    // [[Local]] answers `undefined` rather than throwing when storage is refused —
    // some private-browsing modes do — so only the offline copy is ever lost.
    const stored: unknown = await this.store.get(`kept-${id}`);
    if (typeof stored !== 'object' || stored === null || !('entries' in stored)) return [];
    const { entries } = stored;
    return Array.isArray(entries) ? entries.filter(isEntry) : [];
  }

  /**
   * The messages that were waiting to enter that conversation, kept with it: a
   * copy of one without the other drops a message on its way between them.
   */
  async listed(id: string): Promise<Listed | undefined> {
    const stored: unknown = await this.store.get(`kept-${id}`);
    if (typeof stored !== 'object' || stored === null || !('listed' in stored)) return undefined;
    return isListed(stored.listed) ? stored.listed : undefined;
  }

  /**
   * Keep the end of this conversation and what waits to enter it, at most every
   * [EVERY] milliseconds. Throttled rather than written on leaving: a phone stops
   * by the tunnel dropping or the app being killed, and neither runs code here.
   */
  keep(id: string, entries: Entry[], listed?: Listed): void {
    // Nothing is not worth keeping, and keeping it costs the real copy: the first
    // event of a stream arrives before anything is folded and would win the throttle.
    if (!entries.length) return;
    const now = Date.now();
    if (now - (this.lastWrote.get(id) ?? 0) < Kept.EVERY) return;
    this.lastWrote.set(id, now);
    void this.write(id, entries, listed);
  }

  /** Keep it now, whatever the throttle says. For leaving a session. */
  keepNow(id: string, entries: Entry[], listed?: Listed): void {
    if (!entries.length) return;
    this.lastWrote.set(id, Date.now());
    void this.write(id, entries, listed);
  }

  private async write(id: string, entries: Entry[], listed?: Listed): Promise<void> {
    await this.store.set(`kept-${id}`, { entries: entries.slice(-Kept.ENTRIES), listed });
  }
}

const text = (value: object): boolean => 'text' in value && typeof value.text === 'string';

/**
 * What each kind of entry must carry to be drawable. A table, not a `switch`, so
 * `satisfies` fails when an `Entry` kind is added without a rule; a switch's
 * default would drop that kind silently, and only on a phone that had been offline.
 */
const DRAWABLE = {
  shown: (value) =>
    'pictures' in value &&
    Array.isArray(value.pictures) &&
    value.pictures.every((name) => typeof name === 'string'),
  said: text,
  asked: text,
  turn: text,
  note: text,
  day: text,
  tool: (value) => text(value) && 'tool' in value && typeof value.tool === 'string',
  ask: (value) => text(value) && 'tool' in value && typeof value.tool === 'string',
} satisfies Record<Entry['kind'], (value: object) => boolean>;

/** The same table, keyed by any string: an alias, not an assertion, so it stays checked. */
const DRAWABLE_BY_KIND: Readonly<Record<string, ((value: object) => boolean) | undefined>> =
  DRAWABLE;

const STANDS: readonly string[] = [
  'held',
  'unread',
  'read',
  'dropped',
] satisfies Waiting['stands'][];

/** Whether a revived value is a list this app can draw, message by message. */
function isListed(value: unknown): value is Listed {
  if (typeof value !== 'object' || value === null) return false;
  if (!('taken' in value) || typeof value.taken !== 'number') return false;
  if (!('messages' in value) || !Array.isArray(value.messages)) return false;
  const messages: unknown[] = value.messages;
  return messages.every(
    (one) =>
      typeof one === 'object' &&
      one !== null &&
      'id' in one &&
      typeof one.id === 'string' &&
      text(one) &&
      'at' in one &&
      typeof one.at === 'number' &&
      'stands' in one &&
      typeof one.stands === 'string' &&
      STANDS.includes(one.stands),
  );
}

/** Whether a revived value is an entry this app can draw. */
function isEntry(value: unknown): value is Entry {
  if (typeof value !== 'object' || value === null || !('kind' in value)) return false;
  if (typeof value.kind !== 'string') return false;
  return DRAWABLE_BY_KIND[value.kind]?.(value) ?? false;
}
