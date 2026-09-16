import { describe, expect, it, vi } from 'vitest';
import { createRxDatabase, type RxCollection } from 'rxdb';
import { getRxStorageMemory } from 'rxdb/plugins/storage-memory';

import { DRAFT_SCHEMA, type Clash, type DraftDoc, draftConflicts, replicate } from './console-db';

const doc = (over: Partial<DraftDoc> = {}): DraftDoc => ({
  ulid: 's1',
  text: 'words',
  at: 1000,
  rev: 3,
  _deleted: false,
  ...over,
});

describe('draftConflicts', () => {
  /**
   * ⚠ **The trap that cost life a silent push-loss on 2026-07-03.**
   *
   * Revisions are minted by the runner, so a local edit changes the TEXT and
   * leaves the revision alone. An `isEqual` that compares revisions judges every
   * such edit already-replicated, and the push is dropped with nothing said.
   */
  it('does not call an edited draft equal just because the revision matches', () => {
    const { isEqual } = draftConflicts(() => undefined);
    expect(isEqual(doc(), doc(), 'test')).toBe(true);
    expect(isEqual(doc(), doc({ text: 'words, and more' }), 'test')).toBe(false);
  });

  /**
   * ⚠ **The opposite trap, and the one this file shipped with.** A push that
   * lands leaves the runner holding the same text at a HIGHER revision. The next
   * pull delivers it; an `isEqual` that also compares `rev` calls that a
   * conflict, and the screen offers a choice between a text and itself.
   */
  it('calls the same text equal even when the runner has moved the revision on', () => {
    const { isEqual } = draftConflicts(() => undefined);
    expect(isEqual(doc({ rev: 3 }), doc({ rev: 4 }), 'test')).toBe(true);
  });

  /**
   * ⚠ **Where a conflict was NOTICED is half the diagnosis.** A push the runner
   * refused and a pull that found the master had moved present identically on
   * screen — two devices wrote — and the two have different causes. RxDB names
   * it; carrying that through is the difference between a log that explains a
   * clash and one that only confirms it happened.
   */
  it('carries through where the conflict was noticed', async () => {
    let seen: Clash | undefined;
    const { resolve } = draftConflicts((c) => (seen = c));
    await resolve(
      { realMasterState: doc({ text: 'theirs' }), newDocumentState: doc({ text: 'mine' }) },
      'down-check-if-equal-0',
    );
    expect(seen?.where).toBe('down-check-if-equal-0');
  });

  it('separates a tombstone from a live document', () => {
    const { isEqual } = draftConflicts(() => undefined);
    expect(isEqual(doc({ _deleted: true }), doc({ _deleted: false }), 'test')).toBe(false);
  });

  /**
   * ⚠ **The master goes to THEIRS.** Prose cannot be field-merged, and a
   * resolver that silently picks a side destroys writing nobody chose to lose.
   * The server keeps what it has; this device keeps its own text locally and
   * both are put on screen.
   */
  it('leaves the server holding its own text and reports both', async () => {
    const seen: Clash[] = [];
    const { resolve } = draftConflicts((c) => seen.push(c));
    const theirs = doc({ text: 'from the other device', rev: 9 });

    const resolved = await resolve(
      {
        realMasterState: theirs,
        newDocumentState: doc({ text: 'mine, unsent' }),
        assumedMasterState: doc(),
      },
      'test',
    );

    expect(resolved).toEqual(theirs);
    expect(seen).toEqual([{ id: 's1', mine: 'mine, unsent', theirs, where: 'test' }]);
  });
});

/**
 * A runner that speaks the protocol, so the handlers are tested against the
 * shape rather than against a mock of themselves.
 *
 * ⚠ **It judges a push on the assumed TEXT, because the real one does** — see
 * `Drafts::apply` in `console/src/drafts.rs`. This stood in for the runner while
 * comparing `assumedMasterState.rev`, which is what the runner did until
 * 2026-09-16 and is wrong: a revision is minted server-side, so a client that
 * has pushed but not yet pulled still assumes the revision it edited FROM. A
 * mock that keeps the old rule would pass every test here against a server that
 * does not exist.
 */
function fakeRunner() {
  const held = new Map<string, DraftDoc>();
  let next = 0;
  const server = (url: string | URL | Request, init?: RequestInit): Promise<Response> => {
    const href = typeof url === 'string' ? url : url instanceof URL ? url.href : url.url;
    if (init?.method === 'POST') {
      const rows = JSON.parse(typeof init.body === 'string' ? init.body : '[]') as {
        newDocumentState: DraftDoc;
        assumedMasterState?: DraftDoc;
      }[];
      const lost: DraftDoc[] = [];
      for (const row of rows) {
        const current = held.get(row.newDocumentState.ulid);
        if (current && current.text !== row.assumedMasterState?.text) {
          lost.push(current);
          continue;
        }
        held.set(row.newDocumentState.ulid, { ...row.newDocumentState, rev: ++next });
      }
      return Promise.resolve(new Response(JSON.stringify(lost), { status: 200 }));
    }
    const since = Number(new URL(href, 'http://x').searchParams.get('since') ?? 0);
    const documents = [...held.values()].filter((d) => d.rev > since).sort((a, b) => a.rev - b.rev);
    const rev = documents.at(-1)?.rev ?? since;
    return Promise.resolve(
      new Response(JSON.stringify({ documents, checkpoint: { rev } }), { status: 200 }),
    );
  };
  return { held, server: vi.fn(server) as typeof fetch };
}

async function collection(): Promise<RxCollection<DraftDoc>> {
  const db = await createRxDatabase({
    name: `t${Math.random().toString(36).slice(2)}`,
    storage: getRxStorageMemory(),
  });
  const added = await db.addCollections({
    drafts: { schema: DRAFT_SCHEMA, conflictHandler: draftConflicts(() => undefined) },
  });
  return added.drafts;
}

describe('replicate', () => {
  /** A protocol mismatch is SILENT — replication simply never converges — so the
   *  wire shape is worth a test of its own. */
  it('sends a local draft to the runner', async () => {
    const runner = fakeRunner();
    const drafts = await collection();
    const state = replicate(drafts, runner.server);
    await drafts.insert({ ulid: 's1', text: 'typed here', at: 1, rev: 0, _deleted: false });
    await state.awaitInSync();
    expect(runner.held.get('s1')?.text).toBe('typed here');
  });

  it('takes a draft the runner already holds', async () => {
    const runner = fakeRunner();
    runner.held.set('s2', { ulid: 's2', text: 'from elsewhere', at: 2, rev: 1, _deleted: false });
    const drafts = await collection();
    const state = replicate(drafts, runner.server);
    await state.awaitInSync();
    const got = await drafts.findOne('s2').exec();
    expect(got?.text).toBe('from elsewhere');
  });
});
