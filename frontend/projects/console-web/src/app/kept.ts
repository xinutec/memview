import { Injectable, inject } from '@angular/core';

import { ConsoleDb } from './console-db';
import type { Entry } from './models';

/**
 * The last transcript this phone saw, so a session can be READ when the Mac
 * cannot be reached.
 *
 * ⚠ **Read, and nothing else.** Nothing here is ever sent, retried or acted on.
 * The other half of the Life app's offline working — queueing a message to go
 * when the connection returns — was considered and declined (memview #90): a
 * message to a session is an INSTRUCTION, and one delivered minutes later,
 * after the conversation has moved on, is not the thing that was meant. A failed
 * send already keeps its draft in the composer, which is the same work and
 * leaves the decision with the person.
 *
 * ⚠ **A LOCAL document, so replication cannot carry it.** A transcript is this
 * phone's copy of a conversation and has no business on another device.
 *
 * ⚠ **No service worker, deliberately.** The console has none, because it sits
 * behind a client-certificate gate and ngsw's `navigationUrls` and auth are a
 * known source of trouble here — see `reference_ngsw_auth_navigationurls`. So
 * this is plain storage, written while the app runs, and the app itself still
 * has to be loaded from the network. It makes a session readable on a phone
 * whose tunnel has dropped; it does not make the console an offline app.
 */
@Injectable({ providedIn: 'root' })
export class Kept {
  private db = inject(ConsoleDb);

  /**
   * How much of a conversation is kept, in entries.
   *
   * The end of it, because that is what somebody re-opening a session is looking
   * for. Reading further back needs the runner anyway — the pages come from the
   * file on disk, which this phone has never had.
   */
  private static readonly ENTRIES = 200;

  /** How often one session's copy is rewritten, in milliseconds. */
  private static readonly EVERY = 5_000;

  private lastWrote = new Map<string, number>();

  /**
   * What was kept for this session, or nothing.
   *
   * ⚠ **Checked field by field, not cast.** Storage outlives every deploy that
   * touched this phone, so what comes back may have been written by a build two
   * versions gone; a cast would be a claim about code that no longer runs, and
   * the damage would land in the renderer.
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
      // ⚠ **A database that will not open must not take the reader with it.**
      // IndexedDB is refused outright in some private-browsing modes. What is
      // lost is the offline copy; the live conversation is unaffected, which is
      // why this is silent where a draft's failure is not.
      return [];
    }
  }

  /**
   * Keep the end of this conversation, at most every [EVERY] milliseconds.
   *
   * Throttled rather than written on leaving the session, because leaving is not
   * how a phone stops: the tunnel drops, or the app is swapped out and killed,
   * and neither runs any code here. A copy up to five seconds old is the point.
   */
  keep(id: string, entries: Entry[]): void {
    // ⚠ **Nothing is not worth keeping, and keeping it costs the real copy.**
    // The first event of a stream arrives before anything has been folded, so
    // the emptiest call is the one that wins the throttle — and the next five
    // seconds of a conversation are then blocked behind a copy of nothing.
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
      // A copy that cannot be written is not a failure worth propagating: the
      // session is being read live at this moment, which is why there is
      // anything to keep.
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
  if (typeof value !== 'object' || value === null) return false;
  if (!('kind' in value) || typeof value.kind !== 'string') return false;
  if (!('text' in value) || typeof value.text !== 'string') return false;
  // `at`, `picture`, `detail` and the rest are optional in the shape and
  // optional here: an entry missing one draws, an entry missing `kind` does not.
  return true;
}
