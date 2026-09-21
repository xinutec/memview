import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { Local } from './local';
import { Drafts } from './drafts';
import { Kept } from './kept';
import type { Entry } from './models';

const said = (text: string): Entry => ({ kind: 'said', text });

/**
 * A store on a database of its own.
 *
 * Against a REAL IndexedDB, provided by `fake-indexeddb` in
 * `src/test-setup.ts`. jsdom has none, and [[Local]] is deliberately quiet when
 * storage is refused — so without that setup every assertion here would read back
 * nothing and agree with itself.
 */
function kept(): { store: Kept } {
  TestBed.resetTestingModule();
  TestBed.inject(Local).under(`t${Math.random().toString(36).slice(2)}`);
  return { store: TestBed.inject(Kept) };
}

/** `keep` is fire-and-forget, so a reader waits for it rather than guessing. */
function settled(store: Kept, id: string, count: number): Promise<void> {
  return vi.waitFor(async () => expect((await store.entries(id)).length).toBe(count));
}

afterEach(() => {
  TestBed.inject(Local).close();
});

describe('Kept', () => {
  it('gives back what it was given', async () => {
    const { store } = kept();
    store.keepNow('s1', [said('hello'), said('there')]);
    await settled(store, 's1', 2);
    expect(await store.entries('s1')).toEqual([said('hello'), said('there')]);
  });

  it('knows nothing about a session it never kept', async () => {
    const { store } = kept();
    expect(await store.entries('never-seen')).toEqual([]);
  });

  /**
   * Nothing is not worth keeping, and keeping it costs the real copy. The
   * first event of a stream arrives before anything has been folded, so the
   * emptiest call is the one that would win the throttle — blocking the next
   * five seconds of the conversation behind a copy of nothing.
   */
  it('does not let an empty copy take the throttle', async () => {
    const { store } = kept();
    store.keep('s1', []);
    store.keep('s1', [said('the real thing')]);
    await settled(store, 's1', 1);
    expect(await store.entries('s1')).toEqual([said('the real thing')]);
  });

  /** What somebody re-opening a session wants is what was just said. Reading
   *  further back needs the runner anyway. */
  it('keeps the END of a long conversation, not the beginning', async () => {
    const { store } = kept();
    store.keepNow(
      's1',
      Array.from({ length: 500 }, (_, n) => said(`line ${n}`)),
    );
    await settled(store, 's1', 200);
    expect((await store.entries('s1')).at(-1)).toEqual(said('line 499'));
  });

  /**
   * Checked field by field, not cast. Storage outlives every deploy that
   * touched this phone, so a row may have been written by a build two versions
   * gone — and the damage from a cast lands in the renderer, not here.
   */
  it('reads past anything that is not a transcript', async () => {
    const { store } = kept();
    const local = TestBed.inject(Local);

    await local.set('kept-s1', { entries: 'not an array' });
    expect(await store.entries('s1')).toEqual([]);

    await local.set('kept-s2', { nothing: 'of the sort' });
    expect(await store.entries('s2')).toEqual([]);

    await local.set('kept-s3', { entries: [{ text: 'no kind' }, { kind: 'said', text: 'ok' }] });
    expect(await store.entries('s3')).toEqual([said('ok')]);
  });

  /**
   * A transcript is this phone's copy and must not reach another device.
   * It is kept in [[Local]], which has no sync of any kind — where the draft it
   * sits beside is a document the runner is told about. Nothing here can leak by
   * being written to the wrong store, because the other store is not a store.
   */
  it('is kept where nothing syncs it', async () => {
    const { store } = kept();
    store.keepNow('s1', [said('something private')]);
    await settled(store, 's1', 1);

    const drafts = TestBed.inject(Drafts);
    const sent: string[] = [];
    drafts.configure(
      vi.fn((_url: string | URL | Request, init?: RequestInit) => {
        if (typeof init?.body === 'string') sent.push(init.body);
        return Promise.resolve(
          new Response('{"documents":[],"checkpoint":{"rev":0}}', { status: 200 }),
        );
      }),
    );
    drafts.sync();
    await vi.waitFor(() => expect(sent.length).toBeGreaterThanOrEqual(0));
    expect(sent.join(''), 'a kept transcript reached the runner').not.toContain(
      'something private',
    );
  });
});
