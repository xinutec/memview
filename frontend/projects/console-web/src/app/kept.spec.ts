import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { getRxStorageMemory } from 'rxdb/plugins/storage-memory';

import { ConsoleDb } from './console-db';
import { Kept } from './kept';
import type { Entry } from './models';

const said = (text: string): Entry => ({ kind: 'said', text });

const opened: ConsoleDb[] = [];

/** A store on a memory database of its own. */
async function kept(): Promise<{ store: Kept; db: ConsoleDb }> {
  TestBed.resetTestingModule();
  const db = TestBed.inject(ConsoleDb);
  opened.push(db);
  await db.collection(
    getRxStorageMemory(),
    vi.fn(() =>
      Promise.resolve(
        new Response(JSON.stringify({ documents: [], checkpoint: { rev: 0 } }), { status: 200 }),
      ),
    ),
    `t${Math.random().toString(36).slice(2)}`,
  );
  return { store: TestBed.inject(Kept), db };
}

/** `keep` is fire-and-forget, so a reader waits for it rather than guessing. */
function settled(store: Kept, id: string, count: number): Promise<void> {
  return vi.waitFor(async () => expect((await store.entries(id)).length).toBe(count));
}

afterEach(async () => {
  await Promise.all(opened.splice(0, opened.length).map((db) => db.close()));
});

describe('Kept', () => {
  it('gives back what it was given', async () => {
    const { store } = await kept();
    store.keepNow('s1', [said('hello'), said('there')]);
    await settled(store, 's1', 2);
    expect((await store.entries('s1')).map((e) => e.text)).toEqual(['hello', 'there']);
  });

  it('knows nothing about a session it never kept', async () => {
    const { store } = await kept();
    expect(await store.entries('never-seen')).toEqual([]);
  });

  /**
   * ⚠ **Nothing is not worth keeping, and keeping it costs the real copy.** The
   * first event of a stream arrives before anything has been folded, so the
   * emptiest call is the one that would win the throttle — blocking the next
   * five seconds of the conversation behind a copy of nothing.
   */
  it('does not let an empty copy take the throttle', async () => {
    const { store } = await kept();
    store.keep('s1', []);
    store.keep('s1', [said('the real thing')]);
    await settled(store, 's1', 1);
    expect((await store.entries('s1')).map((e) => e.text)).toEqual(['the real thing']);
  });

  /** What somebody re-opening a session wants is what was just said. Reading
   *  further back needs the runner anyway. */
  it('keeps the END of a long conversation, not the beginning', async () => {
    const { store } = await kept();
    store.keepNow(
      's1',
      Array.from({ length: 500 }, (_, n) => said(`line ${n}`)),
    );
    await settled(store, 's1', 200);
    expect((await store.entries('s1')).at(-1)?.text).toBe('line 499');
  });

  /**
   * ⚠ **Checked field by field, not cast.** Storage outlives every deploy that
   * touched this phone, so a row may have been written by a build two versions
   * gone — and the damage from a cast lands in the renderer, not here.
   */
  it('reads past anything that is not a transcript', async () => {
    const { store, db } = await kept();
    const database = await db.database();

    await database.upsertLocal('kept-s1', { entries: 'not an array' });
    expect(await store.entries('s1')).toEqual([]);

    await database.upsertLocal('kept-s2', { nothing: 'of the sort' });
    expect(await store.entries('s2')).toEqual([]);

    await database.upsertLocal('kept-s3', {
      entries: [{ text: 'no kind' }, { kind: 'said', text: 'ok' }],
    });
    expect((await store.entries('s3')).map((e) => e.text)).toEqual(['ok']);
  });

  it('forgets a conversation on request', async () => {
    const { store } = await kept();
    store.keepNow('s1', [said('hello')]);
    await settled(store, 's1', 1);
    await store.forget('s1');
    expect(await store.entries('s1')).toEqual([]);
  });

  /**
   * ⚠ **A transcript is this phone's copy and must not reach another device.**
   * A local document cannot replicate — that is the reason for using one.
   */
  it('is kept outside the replicated collection', async () => {
    const { store, db } = await kept();
    store.keepNow('s1', [said('something private')]);
    await settled(store, 's1', 1);

    const collection = await db.collection();
    const rows = await collection.find().exec();
    expect(JSON.stringify(rows.map((r) => r.toJSON()))).not.toContain('something private');
  });
});
