import { Injectable } from '@angular/core';

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
 * ⚠ **No service worker, deliberately.** The console has none, because it sits
 * behind a client-certificate gate and ngsw's `navigationUrls` and auth are a
 * known source of trouble here — see `reference_ngsw_auth_navigationurls`. So
 * this is plain storage, written while the app runs, and the app itself still
 * has to be loaded from the network. It makes a session readable on a phone
 * whose tunnel has dropped; it does not make the console an offline app.
 */
@Injectable({ providedIn: 'root' })
export class Kept {
  private static readonly PREFIX = 'console.kept.';

  /**
   * How much of a conversation is kept, in entries.
   *
   * The end of it, because that is what somebody re-opening a session is
   * looking for. Reading further back needs the runner anyway — the pages come
   * from the file on disk, which this phone has never had.
   */
  private static readonly ENTRIES = 200;

  /**
   * The ceiling on one session's kept copy, in characters of JSON.
   *
   * ⚠ **Because this shares an origin's storage with the drafts**, and a draft
   * holds a scaled picture as a data URL — the one thing here that can be a
   * megabyte on its own. A transcript that filled the quota would take the
   * unsent message down with it, which is a worse loss than not being able to
   * read a conversation offline: the draft is the only copy of something a
   * person wrote.
   */
  private static readonly ROOM = 256_000;

  /** How often one session's copy is rewritten, in milliseconds. */
  private static readonly EVERY = 5_000;

  private lastWrote = new Map<string, number>();

  /**
   * What was kept for this session, or nothing.
   *
   * ⚠ **Checked field by field, not cast.** Storage outlives every deploy that
   * touched this phone, so what comes back may have been written by a build two
   * versions gone — the same reason `drafts.ts` revives its picture this way.
   * A `JSON.parse(…) as Entry[]` would be a claim about code that no longer
   * runs, and the damage would land in the renderer rather than here.
   */
  entries(id: string): Entry[] {
    const stored = localStorage.getItem(`${Kept.PREFIX}${id}`);
    if (!stored) return [];
    try {
      const value: unknown = JSON.parse(stored);
      if (!Array.isArray(value)) return [];
      return value.filter(isEntry);
    } catch {
      // Unreadable is simply nothing kept. There is no version of this worth
      // reporting: the conversation is on the Mac, and the copy was a courtesy.
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
    const now = Date.now();
    if (now - (this.lastWrote.get(id) ?? 0) < Kept.EVERY) return;
    this.lastWrote.set(id, now);
    this.write(id, entries);
  }

  /** Keep it now, whatever the throttle says. For leaving a session. */
  keepNow(id: string, entries: Entry[]): void {
    this.lastWrote.set(id, Date.now());
    this.write(id, entries);
  }

  private write(id: string, entries: Entry[]): void {
    let end = entries.slice(-Kept.ENTRIES);
    let text = JSON.stringify(end);
    // Drop from the FRONT until it fits: the newest is what is wanted, and a
    // copy cut at the end would keep the beginning of a conversation and lose
    // what was just said.
    while (text.length > Kept.ROOM && end.length > 1) {
      end = end.slice(Math.ceil(end.length / 10));
      text = JSON.stringify(end);
    }
    try {
      localStorage.setItem(`${Kept.PREFIX}${id}`, text);
    } catch {
      // A full quota is not a failure worth propagating — the session is being
      // read live at this moment, which is why there is anything to keep. Drop
      // this session's copy so the space goes back rather than leaving a stale
      // one that will never be rewritten.
      this.forget(id);
    }
  }

  /** Throw away what was kept — for a conversation that is gone. */
  forget(id: string): void {
    localStorage.removeItem(`${Kept.PREFIX}${id}`);
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
