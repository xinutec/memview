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

/**
 * One conversation's unsent words, as they travel. Mirrors `DraftDoc` in
 * `console/src/drafts.rs`.
 */
export interface DraftDoc {
  /**
   * The SESSION id: a draft is one per conversation, which already has a stable
   * identity.
   */
  ulid: string;
  text: string;
  at: number;
  /**
   * RxDB's tombstone flag, always present and always false: a cleared draft is a
   * live document with empty text, so the other device cannot push back a message
   * already sent. Required because RxDB's `WithDeleted` demands it.
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
    // A session id is a uuid; the length is RxDB's requirement for a primary key.
    ulid: { type: 'string', maxLength: 64 },
    text: { type: 'string' },
    at: { type: 'number' },
    rev: { type: 'number' },
    _deleted: { type: 'boolean' },
  },
  required: ['ulid', 'text', 'at', 'rev'],
};

// Registered once, at module load, and statically imported: no dynamic loading
// anywhere in this app.
addRxPlugin(RxDBLocalDocumentsPlugin);

/**
 * The database this console keeps its local state in. RxDB refuses a second
 * database of the same name in one process, so a test passes its own.
 */
export const CONSOLE_DATABASE = 'consoledrafts';

/** How often to ask the runner whether anything is new. */
const PULL_EVERY_MS = 5000;

/**
 * The batch, shape-checked. The row TYPE is our own contract; the SHAPE is not:
 * a non-array `documents` reaches RxDB as a batch, and a missing
 * `checkpoint.rev` rewinds the pull to zero for ever, silently.
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
   * refused, or a pull that found the master moved. For the LOG: a clash is the
   * one outcome nobody can diagnose from the phone.
   */
  readonly where: string;
}

/**
 * How a collision is settled.
 *
 * `isEqual` compares content and NOTHING else: `rev` alone judges every local
 * edit already-replicated, and beside the text it calls the runner's own echo a
 * conflict. Both are pinned by tests.
 *
 * `resolve` gives the master to THEIRS: prose cannot be field-merged, so both go
 * on screen and a person decides.
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
 * The draft collection and its replication against the runner. Not what the
 * composer reads — typing must not wait for a database — so [[Drafts]] seeds the
 * composer from local storage first and this sits behind it, owning the
 * checkpoint, the retry and the conflict detection.
 */
@Injectable({ providedIn: 'root' })
export class ConsoleDb {
  private readonly telemetry = inject(Telemetry);
  private opened?: Promise<RxCollection<DraftDoc>>;
  private running?: ReturnType<typeof replicate>;
  readonly clash = signal<Clash | undefined>(undefined);

  /**
   * The collection, created once; concurrent callers share one promise, since
   * `createRxDatabase` with the same name throws on a second call.
   */
  collection(
    storage: RxStorage<unknown, unknown> = getRxStorageDexie(),
    get: typeof fetch = fetch,
    name = CONSOLE_DATABASE,
  ): Promise<RxCollection<DraftDoc>> {
    // The arguments are read ONLY on the call that opens it, so a later caller
    // cannot rename a database already open or reach for the real one from a test.
    if (!this.opened) {
      this.opened = this.open(storage, get, name);
      // A memoised promise is unhandled from the moment it is made: if opening fails
      // (some private-browsing modes refuse IndexedDB) the rejection is loose before
      // any caller attaches. They still see it.
      this.opened.catch(() => undefined);
    }
    return this.opened;
  }

  /**
   * The database, for state that is NOT replicated — a kept transcript, a held
   * picture. RxDB excludes local documents from replication itself.
   */
  async database(): Promise<RxDatabase> {
    return (await this.collection()).database;
  }

  /**
   * Ask the runner now rather than waiting out the interval, for the two moments
   * somebody is certainly looking: opening a conversation, and coming back to the
   * front — see [[Foreground]]. A phone taken out of a pocket fires no `online`.
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
   * Raise a clash, and SAY SO — lengths and times, never the words: this reaches
   * `adb logcat` and the fleet trace.
   */
  private raise(clash: Clash): void {
    this.clash.set(clash);
    const age = Math.round((Date.now() - clash.theirs.at) / 1000);
    const said =
      `draft clash on ${clash.id.slice(0, 8)}: ${clash.mine.length} char(s) here ` +
      `against ${clash.theirs.text.length} at rev ${clash.theirs.rev}, written ${age}s ago ` +
      `(noticed by ${clash.where})`;
    // Both: the trace is the only one readable without the phone on a cable.
    console.warn(said);
    this.telemetry.note('draft-clash', said);
  }

  /**
   * Stop replicating and close the database. Cancel the replication FIRST: it
   * holds a subscription and a retry timer.
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
      // Enabled on the DATABASE as well as the collection: a kept transcript belongs
      // to this console rather than to the drafts.
      localDocuments: true,
    });
    const added = await db.addCollections({
      drafts: {
        schema: DRAFT_SCHEMA,
        conflictHandler: draftConflicts((c) => this.raise(c)),
        // Where the held picture lives; RxDB excludes local documents from replication.
        localDocuments: true,
      },
    });
    this.running = replicate(added.drafts, get);
    // Said out loud, because replication failing is SILENT: a dead tunnel is the
    // ordinary case, and a handler that throws on every cycle looks exactly like a
    // quiet one from the composer.
    this.running.error$.subscribe((err: unknown) => {
      console.warn('draft replication:', err instanceof Error ? err.message : err);
    });
    return added.drafts;
  }
}

/**
 * Pull and push against `/api/sync/drafts`. Exported and taking its own `fetch`
 * so the wire shape can be tested without a runner — a mismatch is silent;
 * replication simply never converges.
 */
export function replicate(
  collection: RxCollection<DraftDoc>,
  get: typeof fetch,
): ReturnType<typeof replicateRxCollection<DraftDoc, { rev: number }>> {
  // Ask again on a timer, and at once on coming back from offline.
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
        // Two things replication cannot survive being wrong about: a non-array
        // `documents`, and a missing `checkpoint.rev`, which rewinds the pull to 0 for ever.
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
        // An EMPTY answer means every row landed: a conflicting push is a successful
        // request carrying the current master. `_deleted` is filled in rather than
        // trusted — the runner never writes it, and RxDB's type requires it.
        const lost = asDocs({ documents: (await res.json()) as unknown });
        if (lost === null) throw new Error('draft push returned a malformed answer');
        return lost;
      },
    },
  });
}
