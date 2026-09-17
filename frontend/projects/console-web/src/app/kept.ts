import { Injectable, inject } from '@angular/core';

import { ConsoleDb } from './console-db';
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
  private db = inject(ConsoleDb);

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
    try {
      const db = await this.db.database();
      const held = await db.getLocal(`kept-${id}`);
      const stored: unknown = held?.toJSON().data;
      if (typeof stored !== 'object' || stored === null || !('entries' in stored)) return [];
      const { entries } = stored;
      return Array.isArray(entries) ? entries.filter(isEntry) : [];
    } catch {
      // A database that will not open must not take the reader with it: IndexedDB is
      // refused in some private-browsing modes, and only the offline copy is lost.
      return [];
    }
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
    try {
      const db = await this.db.database();
      await db.upsertLocal(`kept-${id}`, { entries: entries.slice(-Kept.ENTRIES) });
    } catch {
      // A copy that cannot be written is not worth propagating: the session is being
      // read live at this moment.
    }
  }

  /** Throw away what was kept — for a conversation that is gone. */
  async forget(id: string): Promise<void> {
    try {
      const db = await this.db.database();
      await (await db.getLocal(`kept-${id}`))?.remove();
    } catch {
      // Nothing kept is nothing to throw away — see [[entries]].
    }
  }
}

/** Whether a revived value is an entry this app can draw. */
function isEntry(value: unknown): value is Entry {
  if (typeof value !== 'object' || value === null || !('kind' in value)) return false;
  switch (value.kind) {
    case 'shown':
      return 'picture' in value && typeof value.picture === 'string';
    case 'said':
    case 'asked':
    case 'turn':
    case 'note':
    case 'day':
      return 'text' in value && typeof value.text === 'string';
    case 'tool':
    case 'ask':
      return (
        'text' in value &&
        typeof value.text === 'string' &&
        'tool' in value &&
        typeof value.tool === 'string'
      );
    default:
      return false;
  }
}
