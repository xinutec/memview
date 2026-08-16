import { beforeEach, describe, expect, it } from 'vitest';

import { Kept } from './kept';
import type { Entry } from './models';

const said = (text: string): Entry => ({ kind: 'said', text });

/** The service without Angular's injector, which it does not use. */
const kept = () => new Kept();

describe('Kept', () => {
  beforeEach(() => localStorage.clear());

  it('gives back what it was given', () => {
    const store = kept();
    store.keepNow('s1', [said('hello'), said('there')]);
    expect(store.entries('s1').map((e) => e.text)).toEqual(['hello', 'there']);
  });

  it('knows nothing about a session it never kept', () => {
    expect(kept().entries('never-seen')).toEqual([]);
  });

  it('keeps the END of a long conversation, not the beginning', () => {
    // What somebody re-opening a session wants is what was just said. Reading
    // further back needs the runner anyway — those pages come from a file on
    // disk this phone has never had.
    const store = kept();
    const many = Array.from({ length: 500 }, (_, n) => said(`line ${n}`));
    store.keepNow('s1', many);
    const back = store.entries('s1');
    expect(back.length).toBeLessThanOrEqual(200);
    expect(back.at(-1)?.text, 'the newest entry was dropped').toBe('line 499');
  });

  it('cuts a copy that would not fit, from the front', () => {
    // ⚠ It shares an origin's storage with the drafts, and a draft holds a
    // scaled picture as a data URL. A transcript that filled the quota would
    // take an unsent message down with it, which is the worse loss: the draft is
    // the only copy of something a person wrote.
    const store = kept();
    const fat = Array.from({ length: 200 }, (_, n) => said(`${n} ${'x'.repeat(4_000)}`));
    store.keepNow('s1', fat);
    const back = store.entries('s1');
    expect(JSON.stringify(back).length).toBeLessThanOrEqual(256_000);
    expect(back.length, 'everything was thrown away').toBeGreaterThan(0);
    expect(back.at(-1)?.text.startsWith('199 '), 'the newest went').toBe(true);
  });

  it('throttles, so a busy stream does not write on every event', () => {
    const store = kept();
    store.keep('s1', [said('first')]);
    store.keep('s1', [said('first'), said('second')]);
    // The second call is inside the window and is skipped, which is the point:
    // a session running tools emits events far faster than this is worth writing.
    expect(store.entries('s1').map((e) => e.text)).toEqual(['first']);
  });

  it('writes anyway when asked to keep now', () => {
    const store = kept();
    store.keep('s1', [said('first')]);
    store.keepNow('s1', [said('first'), said('second')]);
    expect(store.entries('s1')).toHaveLength(2);
  });

  it('treats what a previous build wrote as nothing rather than trusting it', () => {
    // ⚠ Storage outlives every deploy that touched this phone. A cast would be a
    // claim about code that no longer runs, and the damage would land in the
    // renderer — the same reason `drafts.ts` revives its picture field by field.
    localStorage.setItem('console.kept.s1', 'not json at all');
    expect(kept().entries('s1')).toEqual([]);

    localStorage.setItem('console.kept.s2', '{"not":"an array"}');
    expect(kept().entries('s2')).toEqual([]);

    localStorage.setItem('console.kept.s3', '[{"text":"no kind"},{"kind":"said","text":"ok"}]');
    expect(kept().entries('s3').map((e) => e.text)).toEqual(['ok']);
  });

  it('forgets a conversation on request', () => {
    const store = kept();
    store.keepNow('s1', [said('hello')]);
    store.forget('s1');
    expect(store.entries('s1')).toEqual([]);
  });
});
