import { TestBed } from '@angular/core/testing';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { Drafts } from './drafts';
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

describe('Drafts', () => {
  let drafts: Drafts;

  beforeEach(() => {
    localStorage.clear();
    drafts = TestBed.inject(Drafts);
  });

  it('has nothing to say about a session nobody has written to', () => {
    expect(drafts.text('a')).toBe('');
    expect(drafts.picture('a')).toBeUndefined();
  });

  it('keeps one session unsent message apart from another', () => {
    drafts.put('a', 'the first thing', undefined);
    drafts.put('b', 'the second thing', undefined);
    expect(drafts.text('a')).toBe('the first thing');
    expect(drafts.text('b')).toBe('the second thing');
  });

  it('survives the page being reloaded', () => {
    drafts.put('a', 'half a thought', PICTURE);
    // A reload builds the whole injector again, so a service that only kept a
    // map would come back empty — which is the case this exists for.
    TestBed.resetTestingModule();
    const after = TestBed.inject(Drafts);
    expect(after.text('a')).toBe('half a thought');
    expect(after.picture('a')?.data).toBe(PICTURE.data);
    expect(after.picture('a')?.bytes).toBe(PICTURE.bytes);
  });

  it('gives a revived picture a preview that a reloaded page can show', () => {
    drafts.put('a', '', PICTURE);
    TestBed.resetTestingModule();
    // ⚠ An object URL belongs to the document that made it and is dead in the
    // next one, so a stored `blob:` preview would show as a broken image. The
    // bytes are already here, so the revived preview is a data URL.
    expect(TestBed.inject(Drafts).picture('a')?.preview).toBe(
      `data:${PICTURE.mediaType};base64,${PICTURE.data}`,
    );
  });

  it('forgets a draft that has been sent', () => {
    drafts.put('a', 'said it', PICTURE);
    drafts.put('a', '', undefined);
    TestBed.resetTestingModule();
    const after = TestBed.inject(Drafts);
    expect(after.text('a')).toBe('');
    expect(after.picture('a')).toBeUndefined();
    // Not merely empty in memory — gone, or every session ever written to would
    // keep a picture in a quota this app shares with nothing.
    expect(Object.keys(localStorage)).toEqual([]);
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
    expect(TestBed.inject(Drafts).text('a')).toBe('the words are the cheap half');
  });

  it('refuses a picture written by a version of this app that is gone', () => {
    // Storage outlives every deploy that touched the phone. A draft two builds
    // old that no longer carries a media type must not become an upload of
    // `undefined` — it is simply not a draft any more.
    localStorage.setItem(
      'console.draft.a.picture',
      JSON.stringify({ data: 'aGVsbG8=', width: 100, height: 200, bytes: 5 }),
    );
    TestBed.resetTestingModule();
    expect(TestBed.inject(Drafts).picture('a')).toBeUndefined();
  });

  it('reads past a stored picture that is not a picture', () => {
    // Storage is shared with whatever else runs on this origin, and a half
    // written value survives a kill. Losing the draft is the cost; a session
    // that will not open is not.
    localStorage.setItem('console.draft.a.picture', '{"data":');
    localStorage.setItem('console.draft.a.text', 'still here');
    TestBed.resetTestingModule();
    const after = TestBed.inject(Drafts);
    expect(after.picture('a')).toBeUndefined();
    expect(after.text('a')).toBe('still here');
  });
});
