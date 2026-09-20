import { Injectable, inject } from '@angular/core';

import { Local } from './local';
import type { Entry } from './models';

/**
 * The last transcript this phone saw, so a session can be READ when the Mac
 * cannot be reached — read, and nothing else. Queueing a message to go when the
 * connection returns was declined (memview #90): a message is an INSTRUCTION,
 * and one delivered minutes later is not what was meant.
 *
 * A LOCAL document, so replication cannot carry it. No service worker: the
 * console sits behind a client-certificate gate, and ngsw's `navigationUrls`
 * and auth are a known source of trouble there. The app itself still loads
 * from the network.
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
   * Keep the end of this conversation, at most every [EVERY] milliseconds.
   * Throttled rather than written on leaving: a phone stops by the tunnel
   * dropping or the app being killed, and neither runs code here.
   */
  keep(id: string, entries: Entry[]): void {
    // Nothing is not worth keeping, and keeping it costs the real copy: the first
    // event of a stream arrives before anything is folded and would win the throttle.
    if (!entries.length) return;
    const now = Date.now();
    if (now - (this.lastWrote.get(id) ?? 0) < Kept.EVERY) return;
    this.lastWrote.set(id, now);
    void this.write(id, entries);
  }

  /** Keep it now, whatever the throttle says. For leaving a session. */
  keepNow(id: string, entries: Entry[]): void {
    if (!entries.length) return;
    this.lastWrote.set(id, Date.now());
    void this.write(id, entries);
  }

  private async write(id: string, entries: Entry[]): Promise<void> {
    await this.store.set(`kept-${id}`, { entries: entries.slice(-Kept.ENTRIES) });
  }

  /** Throw away what was kept — for a conversation that is gone. */
  async forget(id: string): Promise<void> {
    await this.store.delete(`kept-${id}`);
  }
}

const text = (value: object): boolean => 'text' in value && typeof value.text === 'string';

/**
 * What each kind of entry must carry to be drawable, one rule per kind.
 *
 * ⚠ **A table rather than a `switch`, so the compiler counts the kinds.** The
 * `satisfies` below fails when an `Entry` variant is added without a rule here,
 * where a switch would simply fall to its default and drop every revived entry
 * of the new kind — silently, and only on a phone that had been offline.
 */
const DRAWABLE = {
  shown: (value) => 'picture' in value && typeof value.picture === 'string',
  said: text,
  asked: text,
  turn: text,
  note: text,
  day: text,
  tool: (value) => text(value) && 'tool' in value && typeof value.tool === 'string',
  ask: (value) => text(value) && 'tool' in value && typeof value.tool === 'string',
} satisfies Record<Entry['kind'], (value: object) => boolean>;

/**
 * The same table under the type a WIRE key needs: any string, and a miss.
 * An alias rather than an assertion — the assignment is checked, so the table
 * stays the thing that decides which kinds exist.
 */
const DRAWABLE_BY_KIND: Readonly<Record<string, ((value: object) => boolean) | undefined>> =
  DRAWABLE;

/** Whether a revived value is an entry this app can draw. */
function isEntry(value: unknown): value is Entry {
  if (typeof value !== 'object' || value === null || !('kind' in value)) return false;
  if (typeof value.kind !== 'string') return false;
  return DRAWABLE_BY_KIND[value.kind]?.(value) ?? false;
}
