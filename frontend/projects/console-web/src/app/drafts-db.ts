import { Injectable, signal } from '@angular/core';
import { EMPTY, fromEvent, interval, map, merge, type Observable } from 'rxjs';
import {
  createRxDatabase,
  type RxCollection,
  type RxConflictHandler,
  type RxJsonSchema,
  type RxStorage,
} from 'rxdb';
import { getRxStorageDexie } from 'rxdb/plugins/storage-dexie';
import { replicateRxCollection } from 'rxdb/plugins/replication';

/** One conversation's unsent words, as they travel. Mirrors `DraftDoc` in
 *  `console/src/drafts.rs`; the two are one contract and move together. */
export interface DraftDoc {
  /** The SESSION id. Every other collection in the fleet mints a ulid per row;
   *  a draft is one per conversation, which already has a stable identity. */
  ulid: string;
  text: string;
  at: number;
  /**
   * RxDB's tombstone flag, always present and always false.
   *
   * ⚠ **A cleared draft is a live document with empty text, never a deletion.**
   * The entry has to survive so the other device cannot push back a message
   * already sent — which is the reverse of what this flag means everywhere else.
   * Required rather than optional because RxDB's `WithDeleted` demands it on
   * every row it is handed, and the runner always writes it.
   */
  _deleted: boolean;
  /** The server revision. Minted by the runner, never by a local edit. */
  rev: number;
}

export const DRAFT_SCHEMA: RxJsonSchema<DraftDoc> = {
  version: 0,
  primaryKey: 'ulid',
  type: 'object',
  properties: {
    // A session id is a uuid; the length is RxDB's requirement for a primary
    // key, not a claim about the format.
    ulid: { type: 'string', maxLength: 64 },
    text: { type: 'string' },
    at: { type: 'number' },
    rev: { type: 'number' },
    _deleted: { type: 'boolean' },
  },
  required: ['ulid', 'text', 'at', 'rev'],
};

/** How often to ask the runner whether anything is new. */
const PULL_EVERY_MS = 5000;

/**
 * The batch, with the two things checked that replication cannot survive being
 * wrong about.
 *
 * ⚠ **The ROW type is our own wire contract and is taken on trust; the SHAPE is
 * not.** A non-array `documents` would be handed to RxDB as a batch, and a
 * missing `checkpoint.rev` rewinds the pull to zero and refetches everything on
 * every cycle for ever, saying nothing. Same division life draws, for the same
 * reason — a per-field check on a row whose type this repo defines buys nothing
 * a compiler does not already give.
 */
function asDocs(body: unknown): DraftDoc[] | null {
  if (typeof body !== 'object' || body === null || !('documents' in body)) return null;
  const { documents } = body;
  if (!Array.isArray(documents)) return null;
  // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion
  return (documents as DraftDoc[]).map((d) => ({ ...d, _deleted: !!d._deleted }));
}

/** The checkpoint's revision, or null when the answer did not carry one. */
function asRev(body: unknown): number | null {
  if (typeof body !== 'object' || body === null || !('checkpoint' in body)) return null;
  const { checkpoint } = body;
  if (typeof checkpoint !== 'object' || checkpoint === null || !('rev' in checkpoint)) return null;
  const { rev } = checkpoint;
  return typeof rev === 'number' ? rev : null;
}

/** Both texts, when this device and the runner have each moved since they agreed. */
export interface Clash {
  readonly id: string;
  readonly mine: string;
  readonly theirs: DraftDoc;
}

/**
 * How a collision is settled, and what it deliberately does NOT do.
 *
 * ⚠ **`isEqual` compares CONTENT, and NOTHING ELSE — not `rev`, in either
 * direction.** Two mistakes are available here and they are opposites.
 *
 * Comparing only the revision judges every local edit already-replicated and
 * drops the push with nothing said: revisions are minted by the runner, so an
 * edit changes the text and leaves the revision alone. That is life's
 * 2026-07-03 push-loss bug, and a draft has exactly the shape that reproduces
 * it.
 *
 * ⚠ Comparing the revision AS WELL is the bug this file shipped with until
 * 2026-09-16. After a push lands, the runner holds the same text at a HIGHER
 * revision; the next pull delivers it, and a comparison that includes `rev`
 * calls that a conflict and raises a clash between a text and itself. `rev` is
 * the pull cursor, which is a fact about ordering and not about content — the
 * same reason `Drafts::apply` on the runner refuses to judge a push on it.
 *
 * ⚠ **`resolve` gives the master to THEIRS, not to this device.** Prose cannot
 * be field-merged the way a quantity can, and whichever side the resolver picks
 * silently is a piece of writing nobody chose to lose. Letting the server keep
 * what it has means nothing is destroyed: this device still holds its own text
 * locally, both are offered on screen, and a person decides.
 */
export function draftConflicts(onClash: (clash: Clash) => void): RxConflictHandler<DraftDoc> {
  return {
    isEqual: (a, b) => !!a._deleted === !!b._deleted && a.text === b.text,
    resolve: ({ realMasterState, newDocumentState }) => {
      onClash({
        id: realMasterState.ulid,
        mine: newDocumentState.text,
        theirs: realMasterState,
      });
      return Promise.resolve(realMasterState);
    },
  };
}

/**
 * The draft collection and its replication against the runner.
 *
 * ⚠ **This is not what the composer reads.** Typing must not wait for a database
 * and must not stop when the tunnel does, so [[Drafts]] seeds the composer from
 * local storage before first paint and this sits behind it — owning the
 * checkpoint, the retry and the conflict detection, which is the bookkeeping
 * that was hand-written and wrong four times over.
 */
@Injectable({ providedIn: 'root' })
export class DraftsDb {
  private opened?: Promise<RxCollection<DraftDoc>>;
  private running?: ReturnType<typeof replicate>;
  /** Set only by a test, which needs a database name of its own. */
  named?: string;
  readonly clash = signal<Clash | undefined>(undefined);

  /** The collection, created once. Concurrent callers share one promise:
   *  `createRxDatabase` with the same name throws on a second call. */
  collection(
    storage: RxStorage<unknown, unknown> = getRxStorageDexie(),
    get: typeof fetch = fetch,
  ): Promise<RxCollection<DraftDoc>> {
    this.opened ??= this.open(storage, get);
    return this.opened;
  }

  /**
   * Ask the runner now, rather than waiting out the rest of the interval.
   *
   * For the two moments somebody is certainly looking: opening a conversation,
   * and the app coming back to the front — see [[Foreground]]. The heartbeat
   * already covers `online`, but a phone taken out of a pocket fires no such
   * event, and five seconds of a stale composer is five seconds of the wrong
   * words on screen.
   */
  resync(): void {
    void this.collection().then(() => this.running?.reSync());
  }

  /** Put a clash down once it has been settled. */
  settled(): void {
    this.clash.set(undefined);
  }

  /**
   * Stop replicating and close the database.
   *
   * ⚠ **The replication has to be cancelled FIRST.** It holds a live
   * subscription and a retry timer, and closing a database out from under those
   * leaves a handler writing into something that has gone. Nothing in the app
   * calls this — a page closes by going away — but a test opens a database per
   * case, and RxDB refuses the next one once enough are still open.
   */
  async close(): Promise<void> {
    const opened = this.opened;
    const running = this.running;
    this.opened = undefined;
    this.running = undefined;
    if (!opened) return;
    await running?.cancel();
    await (await opened).database.close();
  }

  private async open(
    storage: RxStorage<unknown, unknown>,
    get: typeof fetch,
  ): Promise<RxCollection<DraftDoc>> {
    // ⚠ A name of its own per database, because RxDB refuses a second one with
    // the same name in a process — which is every test after the first.
    const name = this.named ?? 'consoledrafts';
    const db = await createRxDatabase({ name, storage, multiInstance: true });
    const added = await db.addCollections({
      drafts: { schema: DRAFT_SCHEMA, conflictHandler: draftConflicts((c) => this.clash.set(c)) },
    });
    this.running = replicate(added.drafts, get);
    // ⚠ **Said out loud, because replication failing is SILENT.** A dead tunnel
    // is the ordinary case this whole thing exists for and must not be noise —
    // but a handler that throws on every cycle looks exactly like a quiet one
    // from the composer, and the phone's only window is `adb logcat`.
    this.running.error$.subscribe((err: unknown) => {
      console.warn('draft replication:', err instanceof Error ? err.message : err);
    });
    return added.drafts;
  }
}

/**
 * Pull and push against `/api/sync/drafts`.
 *
 * Exported and taking its own `fetch` so the wire shape can be tested without a
 * runner — the handlers are where a protocol mismatch would live, and a mismatch
 * is silent: replication simply never converges.
 */
export function replicate(
  collection: RxCollection<DraftDoc>,
  get: typeof fetch,
): ReturnType<typeof replicateRxCollection<DraftDoc, { rev: number }>> {
  // Ask again on a timer, and at once on coming back from offline rather than
  // waiting out the rest of the interval.
  const heartbeat$: Observable<'RESYNC'> = merge(
    interval(PULL_EVERY_MS),
    typeof window === 'undefined' ? EMPTY : fromEvent(window, 'online'),
  ).pipe(map(() => 'RESYNC' as const));

  return replicateRxCollection<DraftDoc, { rev: number }>({
    collection,
    replicationIdentifier: 'console-drafts-http',
    live: true,
    retryTime: 5000,
    pull: {
      stream$: heartbeat$,
      handler: async (checkpoint) => {
        const since = checkpoint?.rev ?? 0;
        const res = await get(`/api/sync/drafts?since=${since}`);
        if (!res.ok) throw new Error(`draft pull failed: ${res.status}`);
        const body: unknown = await res.json();
        // ⚠ **Two things replication cannot survive being wrong about.** A
        // non-array `documents` is fed to RxDB as a batch; a missing
        // `checkpoint.rev` rewinds the pull to 0 and refetches everything on
        // every cycle, for ever, saying nothing.
        const documents = asDocs(body);
        const rev = asRev(body);
        if (documents === null || rev === null)
          throw new Error('draft pull returned a malformed batch');
        return { documents, checkpoint: { rev } };
      },
    },
    push: {
      handler: async (rows) => {
        const res = await get('/api/sync/drafts', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(rows),
        });
        if (!res.ok) throw new Error(`draft push failed: ${res.status}`);
        // ⚠ An EMPTY answer means every row landed. A push that conflicts is a
        // successful request carrying the current master, not a failed one.
        //
        // `_deleted` is filled in rather than trusted: the runner never writes
        // it (a cleared draft is a live document with empty text), and RxDB's
        // type requires it present on every row it is handed back.
        const lost = asDocs({ documents: (await res.json()) as unknown });
        if (lost === null) throw new Error('draft push returned a malformed answer');
        return lost;
      },
    },
  });
}
