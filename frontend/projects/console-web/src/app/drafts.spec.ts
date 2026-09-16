import { TestBed } from '@angular/core/testing';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { getRxStorageMemory } from 'rxdb/plugins/storage-memory';
import { firstValueFrom, filter } from 'rxjs';

import { Drafts, type Resolution } from './drafts';
import { ConsoleDb, type DraftDoc } from './console-db';
import type { Picture } from './picture';

/** A scaled picture as `shrink` hands one over, small enough to read in a test. */
const PICTURE: Picture = {
  data: 'aGVsbG8=',
  mediaType: 'image/png',
  width: 100,
  height: 200,
  bytes: 5,
  // An object URL, which is what the picker makes and what does NOT survive.
  preview: 'blob:http://localhost/8f0e',
};

/** A runner that answers the protocol and holds nothing. What the RUNNER decides
 *  is tested in Rust — `console/tests/drafts.rs` owns which pushes land. */
function quiet(): typeof fetch {
  return vi.fn((_url: string | URL | Request, init?: RequestInit) =>
    Promise.resolve(
      init?.method === 'POST'
        ? new Response('[]', { status: 200 })
        : new Response(JSON.stringify({ documents: [], checkpoint: { rev: 0 } }), { status: 200 }),
    ),
  );
}

const opened: ConsoleDb[] = [];

function fresh(get: typeof fetch = quiet()): Drafts {
  const db = TestBed.inject(ConsoleDb);
  opened.push(db);
  // A database name of its own: RxDB refuses a second one under the same name.
  void db.collection(getRxStorageMemory(), get, `t${Math.random().toString(36).slice(2)}`);
  return TestBed.inject(Drafts);
}

afterEach(async () => {
  await Promise.all(opened.splice(0, opened.length).map((db) => db.close()));
});

/** The next text the collection reports, skipping the "not read yet" state. */
function settled(drafts: Drafts, id: string): Promise<string | undefined> {
  return firstValueFrom(drafts.text$(id).pipe(filter((t) => t !== undefined)));
}

describe('Drafts', () => {
  let drafts: Drafts;
  let db: ConsoleDb;

  beforeEach(async () => {
    drafts = fresh();
    db = TestBed.inject(ConsoleDb);
    await db.collection();
  });

  it('has nothing to say about a conversation nobody has written to', async () => {
    expect(await settled(drafts, 'nobody')).toBe('');
  });

  it('keeps one conversation unsent message apart from another', async () => {
    await drafts.write('a', 'for a');
    await drafts.write('b', 'for b');
    expect(await settled(drafts, 'a')).toBe('for a');
    expect(await settled(drafts, 'b')).toBe('for b');
  });

  /**
   * ⚠ **Opening a conversation is not a statement about its draft** —
   * memview#89's first live bug. The composer records itself on open with an
   * empty box; a document created for that makes a device that has only LOOKED
   * the first writer, and the device that actually typed is then refused.
   */
  it('writes no document at all for a conversation that was only opened', async () => {
    await drafts.write('a', '');
    const collection = await db.collection();
    expect(await collection.findOne('a').exec()).toBeNull();
  });

  /**
   * ⚠ **But clearing one that EXISTS is a send, and the tombstone is what stops
   * the other device pushing the message back.**
   */
  it('leaves a tombstone when a message is sent', async () => {
    await drafts.write('a', 'the message');
    await drafts.write('a', '');
    const collection = await db.collection();
    const doc = await collection.findOne('a').exec();
    expect(doc, 'a tombstone, not a removal').not.toBeNull();
    expect(doc?.text).toBe('');
  });

  /**
   * ⚠ **The runner's counter is the runner's.** A client minting a `rev` would
   * be inventing an ordering only the runner is authority on; the pull cursor is
   * built from it.
   */
  it('never mints a revision of its own', async () => {
    const collection = await db.collection();
    await collection.upsert({ ulid: 'a', text: 'from elsewhere', at: 1, rev: 7, _deleted: false });
    await drafts.write('a', 'and something of mine');
    expect((await collection.findOne('a').exec())?.rev).toBe(7);
  });

  describe('the picture', () => {
    /**
     * ⚠ **A LOCAL document, so it cannot replicate — by construction, not by a
     * promise in a comment.** #89 settled that a picture does not cross devices:
     * two images have no meaningful combination, and the device that took one is
     * the one that wants it.
     */
    it('is held outside the replicated collection', async () => {
      await drafts.write('a', 'about this');
      await drafts.hold('a', PICTURE);

      const collection = await db.collection();
      const replicated = await collection.findOne('a').exec();
      expect(Object.keys(replicated?.toJSON() ?? {})).not.toContain('picture');
      expect(JSON.stringify(replicated?.toJSON())).not.toContain(PICTURE.data);
      expect(await collection.getLocal('picture-a')).not.toBeNull();
    });

    it('comes back with a preview a reloaded page can show', async () => {
      await drafts.hold('a', PICTURE);
      const held = await firstValueFrom(drafts.picture$('a').pipe(filter(Boolean)));
      expect(held.preview).toBe('data:image/png;base64,aGVsbG8=');
      expect(held.width).toBe(100);
    });

    it('is put down when the composer lets go of it', async () => {
      await drafts.hold('a', PICTURE);
      await drafts.hold('a', undefined);
      const collection = await db.collection();
      expect(await collection.getLocal('picture-a')).toBeNull();
    });

    /** Written by a build that is two versions gone: not a picture, so not
     *  offered as one. */
    it('refuses a stored shape that is not a picture', async () => {
      const collection = await db.collection();
      await collection.upsertLocal('picture-a', { data: 'aGk=' });
      expect(await firstValueFrom(drafts.picture$('a'))).toBeUndefined();
    });
  });

  describe('settling a clash', () => {
    const theirs: DraftDoc = { ulid: 'a', text: 'theirs', at: 1000, rev: 4, _deleted: false };

    it.each([
      ['mine', 'mine'],
      ['theirs', 'theirs'],
      ['mine-first', 'mine\n\ntheirs'],
      ['theirs-first', 'theirs\n\nmine'],
    ])('%s leaves the conversation holding %j', async (how, expected) => {
      db.clash.set({ id: 'a', mine: 'mine', theirs, where: 'test' });
      await drafts.resolve('a', theirs, how as Resolution);
      expect(await settled(drafts, 'a')).toBe(expected);
      expect(drafts.clash()).toBeUndefined();
    });

    /**
     * ⚠ **A clash that is never recorded cannot be diagnosed**, and a trace
     * showing clashes but never how they ended reads as though every one was
     * abandoned. Pinned on the SHAPE — lengths and the choice, never the words,
     * because a draft is a private message and this reaches `adb logcat` and the
     * fleet trace.
     */
    it('says which way it was settled, without putting the words in the log', async () => {
      const secret: DraftDoc = { ...theirs, text: 'brandenburg concerto' };
      db.clash.set({ id: 'a', mine: 'schleswig holstein', theirs: secret, where: 'test' });

      const said = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
      await drafts.resolve('a', secret, 'mine-first');
      const line = said.mock.calls.map((c) => String(c[0])).join(' ');
      said.mockRestore();

      expect(line).toContain('settled as mine-first');
      expect(line, 'a draft is a private message; it must not reach a log').not.toContain(
        'schleswig',
      );
      expect(line).not.toContain('brandenburg');
      expect(line).toMatch(/\d+ here, \d+ there, \d+ kept/);
    });
  });

  describe('with no connection at all', () => {
    /** The case this store exists for — a phone in a tunnel. */
    async function underground(): Promise<Drafts> {
      TestBed.resetTestingModule();
      const store = fresh(vi.fn(() => Promise.reject(new Error('no route to host'))));
      await TestBed.inject(ConsoleDb).collection();
      return store;
    }

    it('keeps every word, and says nothing about the network', async () => {
      const store = await underground();
      await store.write('a', 'written between two stations');
      expect(await settled(store, 'a')).toBe('written between two stations');
      expect(store.clash()).toBeUndefined();
    });

    it('holds them locally, so they go when the tunnel comes back', async () => {
      const store = await underground();
      await store.write('a', 'written between two stations');
      const collection = await TestBed.inject(ConsoleDb).collection();
      expect((await collection.findOne('a').exec())?.text).toBe('written between two stations');
    });
  });
});
