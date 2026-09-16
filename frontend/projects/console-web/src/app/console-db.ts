import { Injectable, inject, signal } from '@angular/core';
import { EMPTY, fromEvent, interval, map, merge, type Observable } from 'rxjs';
import {
  addRxPlugin,
  createRxDatabase,
  type RxCollection,
  type RxConflictHandler,
  type RxJsonSchema,
  type RxDatabase,
  type RxStorage,
} from 'rxdb';
import { getRxStorageDexie } from 'rxdb/plugins/storage-dexie';
import { RxDBLocalDocumentsPlugin } from 'rxdb/plugins/local-documents';
import { replicateRxCollection } from 'rxdb/plugins/replication';

import { Telemetry } from './telemetry';

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

// ⚠ **Registered once, at module load, and statically imported.** No dynamic
// loading anywhere in this app: the whole bundle ships together and the budget is
// raised when it needs to be, rather than split into pieces.
addRxPlugin(RxDBLocalDocumentsPlugin);

/**
 * The database this console keeps its local state in.
 *
 * ⚠ **RxDB refuses a second database of the same name in one process**, so
 * anything opening more than one — a test, chiefly — passes its own.
 */
export const CONSOLE_DATABASE = 'consoledrafts';

/** How often to ask the runner whether anything is new. */
const PULL_EVERY_MS = 5000;

/**
 * The batch, shape-checked.
 *
 * ⚠ **The row TYPE is our own contract and is trusted; the SHAPE is not.** A
 * non-array `documents` reaches RxDB as a batch, and a missing `checkpoint.rev`
 * rewinds the pull to zero and refetches everything for ever, silently.
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
  /**
   * RxDB's own name for where the conflict was detected — a push the runner
   * refused, or a pull that found the master had moved.
   *
   * ⚠ **Carried for the LOG, not for the screen.** A clash is the one outcome
   * here nobody can diagnose from the phone: it says two devices wrote, and
   * cannot say which side went first or which half of replication noticed. Two
   * rounds of these were guessed at from the symptom before anything recorded
   * it.
   */
  readonly where: string;
}

/**
 * How a collision is settled.
 *
 * ⚠ **`isEqual` compares content and NOTHING else.** Bringing `rev` into it
 * fails in both directions, and both are pinned by tests: on its own it judges
 * every local edit already-replicated and drops the push silently; alongside the
 * text it calls the runner's own echo a conflict and offers a choice between a
 * text and itself.
 *
 * ⚠ **`resolve` gives the master to THEIRS, not to this device.** Prose cannot
 * be field-merged, and a resolver that silently picks a side destroys writing
 * nobody chose to lose. The server keeps what it has, this device keeps its own,
 * both go on screen, and a person decides.
 */
export function draftConflicts(onClash: (clash: Clash) => void): RxConflictHandler<DraftDoc> {
  return {
    isEqual: (a, b) => !!a._deleted === !!b._deleted && a.text === b.text,
    resolve: ({ realMasterState, newDocumentState }, where) => {
      onClash({
        id: realMasterState.ulid,
        mine: newDocumentState.text,
        theirs: realMasterState,
        where,
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
export class ConsoleDb {
  private readonly telemetry = inject(Telemetry);
  private opened?: Promise<RxCollection<DraftDoc>>;
  private running?: ReturnType<typeof replicate>;
  readonly clash = signal<Clash | undefined>(undefined);

  /** The collection, created once. Concurrent callers share one promise:
   *  `createRxDatabase` with the same name throws on a second call. */
  collection(
    storage: RxStorage<unknown, unknown> = getRxStorageDexie(),
    get: typeof fetch = fetch,
    name = CONSOLE_DATABASE,
  ): Promise<RxCollection<DraftDoc>> {
    // ⚠ The arguments are read ONLY on the call that opens it. Storing them
    // would let any later caller using the defaults rename a database that is
    // already open, or reach for the real one from a test.
    if (!this.opened) {
      this.opened = this.open(storage, get, name);
      // ⚠ **A memoised promise is unhandled from the moment it is made.** If
      // opening fails — no IndexedDB, which some private-browsing modes refuse —
      // the rejection is loose before any caller has attached, and that is an
      // unhandled rejection whatever the callers then do. They still see it.
      this.opened.catch(() => undefined);
    }
    return this.opened;
  }

  /**
   * The database, for state that is NOT replicated — a kept transcript, a held
   * picture. RxDB excludes local documents from replication itself, so staying
   * on one device is a property of the storage rather than a promise.
   */
  async database(): Promise<RxDatabase> {
    return (await this.collection()).database;
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
    void this.collection()
      .then(() => this.running?.reSync())
      .catch(() => undefined);
  }

  /** Put a clash down once it has been settled. */
  settled(): void {
    this.clash.set(undefined);
  }

  /**
   * Raise a clash, and SAY SO.
   *
   * ⚠ **Lengths and times, never the words.** A draft is a private message, and
   * this reaches `adb logcat` and the fleet trace. Lengths are enough to tell two
   * drafts apart, which is all a diagnosis needs.
   */
  private raise(clash: Clash): void {
    this.clash.set(clash);
    const age = Math.round((Date.now() - clash.theirs.at) / 1000);
    const said =
      `draft clash on ${clash.id.slice(0, 8)}: ${clash.mine.length} char(s) here ` +
      `against ${clash.theirs.text.length} at rev ${clash.theirs.rev}, written ${age}s ago ` +
      `(noticed by ${clash.where})`;
    // Both, deliberately: the console is the phone's only live window, and the
    // trace is the only one that can be read without the phone on a cable.
    console.warn(said);
    this.telemetry.note('draft-clash', said);
  }

  /**
   * Stop replicating and close the database.
   *
   * ⚠ **Cancel the replication FIRST** — it holds a subscription and a retry
   * timer, and closing underneath those leaves a handler writing into nothing.
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
    name: string,
  ): Promise<RxCollection<DraftDoc>> {
    const db = await createRxDatabase({
      name,
      storage,
      multiInstance: true,
      // Enabled on the DATABASE as well as on the collection below: a kept
      // transcript belongs to this console rather than to the drafts.
      localDocuments: true,
    });
    const added = await db.addCollections({
      drafts: {
        schema: DRAFT_SCHEMA,
        conflictHandler: draftConflicts((c) => this.raise(c)),
        // ⚠ **Where the held picture lives.** RxDB excludes local documents
        // from replication itself, so the picture staying on one device is a
        // property of the storage rather than a promise in a comment.
        localDocuments: true,
      },
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
