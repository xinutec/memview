import { TestBed } from '@angular/core/testing';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { getRxStorageMemory } from 'rxdb/plugins/storage-memory';

import { Drafts, type Resolution } from './drafts';
import { DraftsDb, type DraftDoc } from './drafts-db';
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

/**
 * A runner that answers the sync protocol and holds nothing.
 *
 * ⚠ **What the RUNNER decides is tested in Rust** — `console/tests/drafts.rs`
 * owns which pushes land and which are refused, because that is where the rule
 * lives. What is tested here is the half this file is responsible for: the
 * synchronous mirror, the picture, the debounce, and what reaches the composer.
 */
function quiet(): typeof fetch {
  return vi.fn((_url: string | URL | Request, init?: RequestInit) =>
    Promise.resolve(
      init?.method === 'POST'
        ? new Response('[]', { status: 200 })
        : new Response(JSON.stringify({ documents: [], checkpoint: { rev: 0 } }), { status: 200 }),
    ),
  );
}

/**
 * A store on a memory collection of its own.
 *
 * ⚠ **Synchronous, and that is the point.** `DraftsDb` is asked for its
 * collection before `Drafts` is injected, so the promise is already memoised by
 * the time the constructor asks — which lets a test read `text()` the same
 * instant the store exists, the way the composer does on first paint. Awaiting
 * here would test a page nobody sees.
 */
function fresh(get: typeof fetch = quiet()): Drafts {
  const db = TestBed.inject(DraftsDb);
  db.named = `t${Math.random().toString(36).slice(2)}`;
  // ⚠ Kept so it can be CLOSED. RxDB holds every open database in a process
  // registry and refuses the next one with COL23 once they pile up, so a spec
  // that only ever opens them fails partway down the file — with an error about
  // the collection being created, which points nowhere near here.
  opened.push(db);
  void db.collection(getRxStorageMemory(), get);
  return TestBed.inject(Drafts);
}

/** Every store this spec has opened, so `afterEach` can close them. */
const opened: DraftsDb[] = [];

afterEach(async () => {
  await Promise.all(opened.splice(0, opened.length).map((db) => db.close()));
});

describe('Drafts', () => {
  let drafts: Drafts;

  beforeEach(async () => {
    localStorage.clear();
    drafts = fresh();
    await TestBed.inject(DraftsDb).collection();
  });

  it('has nothing to say about a session nobody has written to', () => {
    expect(drafts.text('nobody')).toBe('');
    expect(drafts.picture('nobody')).toBeUndefined();
  });

  it('keeps one session unsent message apart from another', () => {
    drafts.put('a', 'for a', undefined);
    drafts.put('b', 'for b', undefined);
    expect(drafts.text('a')).toBe('for a');
    expect(drafts.text('b')).toBe('for b');
  });

  /**
   * ⚠ **The reason `localStorage` is still here at all.** IndexedDB is async and
   * the composer paints synchronously, so a draft read from the collection would
   * arrive after an empty box was already on screen.
   */
  it('answers before the collection could, so the composer never paints empty', () => {
    drafts.put('a', 'half a thought', undefined);
    TestBed.resetTestingModule();
    // No `await`: a fresh store, asked the instant it exists.
    expect(fresh().text('a')).toBe('half a thought');
  });

  it('gives a revived picture a preview that a reloaded page can show', () => {
    drafts.put('a', 'about this', PICTURE);
    TestBed.resetTestingModule();
    const revived = fresh().picture('a');
    expect(revived?.preview).toBe('data:image/png;base64,aGVsbG8=');
    expect(revived?.width).toBe(100);
  });

  it('forgets a draft that has been sent', () => {
    drafts.put('a', 'the message', undefined);
    drafts.put('a', '', undefined);
    expect(drafts.text('a')).toBe('');
    expect(localStorage.getItem('console.draft.a.text')).toBeNull();
  });

  it('keeps the words when there is no room for the picture', () => {
    // Only the picture is refused — a mock that swallowed every write would
    // prove nothing about which half survived.
    const real = Storage.prototype.setItem.bind(localStorage);
    const full = vi
      .spyOn(Storage.prototype, 'setItem')
      .mockImplementation((key: string, value: string) => {
        if (key.includes('picture')) throw new DOMException('quota', 'QuotaExceededError');
        real(key, value);
      });
    drafts.put('a', 'the words are the cheap half', PICTURE);
    full.mockRestore();
    // In memory either way: what a full quota costs is the reload, not the
    // picture sitting in the composer right now.
    expect(drafts.picture('a')?.data).toBe(PICTURE.data);
    TestBed.resetTestingModule();
    expect(fresh().text('a')).toBe('the words are the cheap half');
  });

  it('refuses a picture written by a version of this app that is gone', () => {
    localStorage.setItem('console.draft.a.picture', JSON.stringify({ data: 'aGk=' }));
    expect(drafts.picture('a')).toBeUndefined();
  });

  it('reads past a stored picture that is not a picture', () => {
    localStorage.setItem('console.draft.a.picture', '{half a wri');
    expect(drafts.picture('a')).toBeUndefined();
  });

  describe('reaching the collection', () => {
    it('writes the words, once the typing has paused', async () => {
      vi.useFakeTimers();
      drafts.put('a', 'typed here', undefined);
      await vi.advanceTimersByTimeAsync(900);
      vi.useRealTimers();

      const collection = await TestBed.inject(DraftsDb).collection();
      expect((await collection.findOne('a').exec())?.text).toBe('typed here');
    });

    /**
     * ⚠ **RxDB pushes per local WRITE, so the debounce is not tidying.** `put`
     * is called per keystroke; without this, a sentence is a request per
     * character.
     */
    it('writes once for a burst of keystrokes, not once per keystroke', async () => {
      vi.useFakeTimers();
      for (const text of ['t', 'ty', 'typ', 'type', 'typed']) drafts.put('a', text, undefined);
      await vi.advanceTimersByTimeAsync(900);
      vi.useRealTimers();

      const collection = await TestBed.inject(DraftsDb).collection();
      const doc = await collection.findOne('a').exec();
      expect(doc?.text).toBe('typed');
      expect(doc?.revision.startsWith('1-')).toBe(true);
    });

    /**
     * ⚠ **The echo of a pull must not be pushed back.** A document that arrives
     * from the other device lands in the mirror, the composer records it, and
     * that record arrives here as an ordinary `put` — so an unchanged text has
     * to write nothing at all or the two devices trade the same words for ever.
     */
    it('writes nothing when the text has not actually changed', async () => {
      const collection = await TestBed.inject(DraftsDb).collection();
      await collection.upsert({
        ulid: 'a',
        text: 'from elsewhere',
        at: 1,
        rev: 3,
        _deleted: false,
      });
      const before = (await collection.findOne('a').exec())?.revision;

      vi.useFakeTimers();
      drafts.put('a', 'from elsewhere', undefined);
      await vi.advanceTimersByTimeAsync(900);
      vi.useRealTimers();

      expect((await collection.findOne('a').exec())?.revision).toBe(before);
    });

    /**
     * ⚠ **Opening a session is not a statement about its draft — memview#89's
     * FIRST live bug, and it came back.**
     *
     * The composer's recording effect runs once on open with whatever is on
     * screen, which is `''` where nothing has been typed here. A document
     * created for that makes a device that has only LOOKED the first writer of
     * the conversation, and the device that actually typed is then refused and
     * shown a clash against nothing. Caught on 2026-09-16 by two browsers
     * against one runner, after the hand-rolled guard was deleted with the rest
     * of the bookkeeping and nothing here noticed.
     */
    it('writes nothing at all for a session that was only opened', async () => {
      vi.useFakeTimers();
      drafts.put('a', '', undefined);
      await vi.advanceTimersByTimeAsync(900);
      vi.useRealTimers();

      const collection = await TestBed.inject(DraftsDb).collection();
      expect(await collection.findOne('a').exec()).toBeNull();
    });

    /** But clearing one that EXISTS is a send, and the tombstone is what stops
     *  the other device pushing the message back. */
    it('still writes the clearing of a draft that is really there', async () => {
      vi.useFakeTimers();
      drafts.put('a', 'the message', undefined);
      await vi.advanceTimersByTimeAsync(900);
      drafts.put('a', '', undefined);
      await vi.advanceTimersByTimeAsync(900);
      vi.useRealTimers();

      const collection = await TestBed.inject(DraftsDb).collection();
      const doc = await collection.findOne('a').exec();
      expect(doc).not.toBeNull();
      expect(doc?.text).toBe('');
    });

    /**
     * ⚠ **OPENING A CONVERSATION THAT ALREADY HOLDS A DRAFT MUST NOT CLEAR IT.**
     *
     * The worst shape of the same bug, found on 2026-09-16 by two browsers
     * against one runner. A device opens the session with an empty box and
     * records that; the draft then arrives from the other device; and the empty
     * record — now no longer a first write, because a document exists — was
     * written straight over it. Merely LOOKING at a conversation on the phone
     * deleted what had been typed on the Mac.
     */
    it('does not clear a draft that arrives just after the box was seeded empty', async () => {
      const collection = await TestBed.inject(DraftsDb).collection();

      // The composer opens and records itself, the way its effect does.
      vi.useFakeTimers();
      drafts.put('a', '', undefined);
      // The other device's words arrive before the debounce has fired.
      await collection.upsert({
        ulid: 'a',
        text: 'typed on the mac',
        at: 1,
        rev: 2,
        _deleted: false,
      });
      await vi.advanceTimersByTimeAsync(900);
      vi.useRealTimers();
      await new Promise((r) => setTimeout(r, 0));

      expect((await collection.findOne('a').exec())?.text).toBe('typed on the mac');
      expect(drafts.text('a')).toBe('typed on the mac');
    });

    /** And the composer recording what it was just HANDED is not a write
     *  either — otherwise every arrival bounces straight back. */
    it('does not write back words that arrived from the other device', async () => {
      const collection = await TestBed.inject(DraftsDb).collection();
      await collection.upsert({
        ulid: 'a',
        text: 'from elsewhere',
        at: 1,
        rev: 2,
        _deleted: false,
      });
      await new Promise((r) => setTimeout(r, 0));
      const settled = (await collection.findOne('a').exec())?.revision;

      vi.useFakeTimers();
      drafts.put('a', 'from elsewhere', undefined);
      await vi.advanceTimersByTimeAsync(900);
      vi.useRealTimers();

      expect((await collection.findOne('a').exec())?.revision).toBe(settled);
    });

    /** ⚠ But a SEND is a clear and must still be written — it is what stops the
     *  other device pushing the message back. The box was handed a draft; an
     *  empty one is not what it was handed. */
    it('still clears when the person has actually sent the message', async () => {
      const collection = await TestBed.inject(DraftsDb).collection();
      await collection.upsert({ ulid: 'a', text: 'the message', at: 1, rev: 2, _deleted: false });
      await new Promise((r) => setTimeout(r, 0));

      vi.useFakeTimers();
      drafts.put('a', '', undefined);
      await vi.advanceTimersByTimeAsync(900);
      vi.useRealTimers();

      expect((await collection.findOne('a').exec())?.text).toBe('');
    });

    /**
     * ⚠ **Throwing a draft away by hand must reach the other device**, and it is
     * the case the echo guard can silently break: the box ends up holding
     * exactly what it was seeded with, so a guard keyed on the SEED rather than
     * on what was last agreed reads a deliberate deletion as an echo and the
     * words stay on the other screen.
     */
    it('clears when the person selects it all and deletes it', async () => {
      const collection = await TestBed.inject(DraftsDb).collection();

      vi.useFakeTimers();
      drafts.put('a', 'something I thought better of', undefined);
      await vi.advanceTimersByTimeAsync(900);
      drafts.put('a', '', undefined);
      await vi.advanceTimersByTimeAsync(900);
      vi.useRealTimers();

      const doc = await collection.findOne('a').exec();
      expect(doc, 'a tombstone, not a removal').not.toBeNull();
      expect(doc?.text).toBe('');
    });

    /**
     * ⚠ **A SENT MESSAGE MUST NOT COME BACK — reported from the phone,
     * 2026-09-16, in the `health` conversation: "press send, it empties, go out
     * and back in, and the old message is in the input again."**
     *
     * It needs the send to land INSIDE the debounce, which is why it is "not
     * always". Typing schedules a write of the words; pressing send empties the
     * composer, and an empty composer against a session nothing has been written
     * for yet reads as an ECHO — so `schedule` is never reached, and `schedule`
     * is the only thing that cancels the pending timer. It then fires and writes
     * the sent words into the collection, after the box was emptied.
     *
     * The rule this pins: whatever the composer holds NOW supersedes anything
     * still waiting to be written, echo or not.
     */
    it('does not write words that were sent before the debounce fired', async () => {
      const collection = await TestBed.inject(DraftsDb).collection();

      vi.useFakeTimers();
      drafts.put('a', 'the message', undefined);
      // Sent well inside WRITE_AFTER_MS, which is the whole condition.
      await vi.advanceTimersByTimeAsync(200);
      drafts.put('a', '', undefined);
      await vi.advanceTimersByTimeAsync(2000);
      vi.useRealTimers();
      await new Promise((r) => setTimeout(r, 0));

      expect(await collection.findOne('a').exec(), 'a sent message was left as a draft').toBeNull();
      expect(drafts.text('a')).toBe('');
    });

    /** The same race with a draft already in the collection: the pending write
     *  carries the LONGER text, and the send must still win. */
    it('does not restore a longer draft that was pending when the message went', async () => {
      const collection = await TestBed.inject(DraftsDb).collection();

      vi.useFakeTimers();
      drafts.put('a', 'the message', undefined);
      await vi.advanceTimersByTimeAsync(900); // this one lands
      drafts.put('a', 'the message, extended', undefined);
      await vi.advanceTimersByTimeAsync(200); // this one is still pending
      drafts.put('a', '', undefined); // sent
      await vi.advanceTimersByTimeAsync(2000);
      vi.useRealTimers();
      await new Promise((r) => setTimeout(r, 0));

      expect((await collection.findOne('a').exec())?.text).toBe('');
      expect(drafts.text('a')).toBe('');
    });

    /**
     * ⚠ **The runner's counter is the runner's.** A client that minted a `rev`
     * would be inventing an ordering only the runner is authority on; the pull
     * cursor is built from it.
     */
    it('carries the runner revision through untouched', async () => {
      const collection = await TestBed.inject(DraftsDb).collection();
      await collection.upsert({
        ulid: 'a',
        text: 'from elsewhere',
        at: 1,
        rev: 7,
        _deleted: false,
      });

      vi.useFakeTimers();
      drafts.put('a', 'and something of mine', undefined);
      await vi.advanceTimersByTimeAsync(900);
      vi.useRealTimers();

      expect((await collection.findOne('a').exec())?.rev).toBe(7);
    });
  });

  describe('while both screens are open', () => {
    async function arrives(text: string): Promise<void> {
      const collection = await TestBed.inject(DraftsDb).collection();
      await collection.upsert({ ulid: 'a', text, at: 2, rev: 4, _deleted: false });
      await new Promise((r) => setTimeout(r, 0));
    }

    it('shows what the other device typed', async () => {
      await arrives('written on the phone');
      expect(drafts.landed()).toEqual({ id: 'a', text: 'written on the phone' });
      expect(drafts.text('a')).toBe('written on the phone');
    });

    /** The thing Pippijn asked for by name: sending on one screen must empty
     *  the other, and a send is a draft cleared. */
    it('empties when the other device SENDS, because a send clears the draft', async () => {
      drafts.put('a', 'about to go', undefined);
      await arrives('');
      expect(drafts.text('a')).toBe('');
      expect(drafts.landed()).toEqual({ id: 'a', text: '' });
    });

    /**
     * ⚠ **A local edit must not announce itself as the other device.** The
     * mirror is written before the collection is, so this device's own echo
     * finds the mirror already equal — which is what `watch` compares and why it
     * never has to ask RxDB where a change came from.
     */
    it('says nothing about this device typing', async () => {
      vi.useFakeTimers();
      drafts.put('a', 'typed right here', undefined);
      await vi.advanceTimersByTimeAsync(900);
      vi.useRealTimers();
      await new Promise((r) => setTimeout(r, 0));

      expect(drafts.landed()).toBeUndefined();
    });
  });

  describe('settling a clash', () => {
    const theirs: DraftDoc = { ulid: 'a', text: 'theirs', at: 1000, rev: 4, _deleted: false };

    beforeEach(() => {
      drafts.put('a', 'mine', undefined);
    });

    /**
     * ⚠ **A clash that is never recorded cannot be diagnosed.** The hand-rolled
     * store said this and the rewrite dropped it; nothing failed, and the gap
     * was found only because Pippijn asked. Pinned so it cannot go quietly
     * again — and pinned on the SHAPE, not the wording: lengths and the choice,
     * never the words, because a draft is a private message and this reaches
     * `adb logcat` and the fleet trace.
     */
    it('says which way a clash was settled, without putting the words in the log', () => {
      // ⚠ Distinctive on both sides. "mine" and "theirs" are the names of the
      // CHOICES as well as the fixture's text, so a log line naming the choice
      // would satisfy a check for the absence of the words by accident.
      const db = TestBed.inject(DraftsDb);
      const secret: DraftDoc = { ...theirs, text: 'brandenburg concerto' };
      db.clash.set({ id: 'a', mine: 'schleswig holstein', theirs: secret, where: 'test' });

      const said = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
      drafts.resolve('a', secret, 'mine-first');
      const line = said.mock.calls.map((c) => String(c[0])).join(' ');
      said.mockRestore();

      expect(line).toContain('settled as mine-first');
      expect(line, 'a draft is a private message; it must not reach a log').not.toContain(
        'schleswig',
      );
      expect(line).not.toContain('brandenburg');
      expect(line).toMatch(/\d+ here, \d+ there, \d+ kept/);
    });

    it.each([
      ['mine', 'mine'],
      ['theirs', 'theirs'],
      ['mine-first', 'mine\n\ntheirs'],
      ['theirs-first', 'theirs\n\nmine'],
    ])('%s leaves the composer holding %j', (how, expected) => {
      drafts.resolve('a', theirs, how as Resolution);
      expect(drafts.text('a')).toBe(expected);
      expect(drafts.clash()).toBeUndefined();
    });

    /**
     * ⚠ **`mine` comes off the CLASH, not off the mirror.** The conflict handler
     * gives the master to theirs, so by the time anybody presses a button the
     * collection — and the mirror behind it — may already hold the other
     * device's words. Reading this device's text from there would settle the
     * clash by discarding the very thing it was raised about.
     */
    it('keeps this device words even after theirs have overwritten the mirror', async () => {
      const db = TestBed.inject(DraftsDb);
      db.clash.set({ id: 'a', mine: 'what I was writing', theirs, where: 'test' });
      const collection = await db.collection();
      await collection.upsert(theirs);
      await new Promise((r) => setTimeout(r, 0));
      expect(drafts.text('a')).toBe('theirs');

      drafts.resolve('a', theirs, 'mine-first');
      expect(drafts.text('a')).toBe('what I was writing\n\ntheirs');
    });
  });

  describe('with no connection at all', () => {
    /** A runner that cannot be reached, which is the case this whole store
     *  exists for — a phone in a tunnel. */
    async function underground(): Promise<Drafts> {
      TestBed.resetTestingModule();
      const store = fresh(vi.fn(() => Promise.reject(new Error('no route to host'))));
      await TestBed.inject(DraftsDb).collection();
      return store;
    }

    it('keeps every word, and says nothing about the network', async () => {
      const store = await underground();
      vi.useFakeTimers();
      store.put('a', 'written between two stations', undefined);
      await vi.advanceTimersByTimeAsync(900);
      vi.useRealTimers();

      expect(store.text('a')).toBe('written between two stations');
      expect(store.clash()).toBeUndefined();
    });

    it('still gets the words into the collection, so they go when the tunnel does', async () => {
      const store = await underground();
      vi.useFakeTimers();
      store.put('a', 'written between two stations', undefined);
      await vi.advanceTimersByTimeAsync(900);
      vi.useRealTimers();

      const collection = await TestBed.inject(DraftsDb).collection();
      expect((await collection.findOne('a').exec())?.text).toBe('written between two stations');
    });

    it('survives the page being destroyed and rebuilt underground', async () => {
      const store = await underground();
      store.put('a', 'written between two stations', undefined);
      TestBed.resetTestingModule();
      expect(fresh().text('a')).toBe('written between two stations');
    });
  });
});
