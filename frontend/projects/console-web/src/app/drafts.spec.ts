import { TestBed } from '@angular/core/testing';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { of, throwError } from 'rxjs';

import { ConsoleApi } from './console-api';
import { Drafts, type Resolution } from './drafts';
import type { StoredDraft } from './models';
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

  /**
   * The runner's draft is taken only when nothing local is at stake.
   *
   * ⚠ Opening a session must not discard what was typed on THIS device in
   * favour of whatever the other one last pushed — that is the failure the
   * clash exists to prevent, and it is silent if it happens.
   */
  describe('meeting the other device', () => {
    const theirs = { text: 'from the other device', rev: 4, at: 1000 };

    /** Answer `GET …/draft` with `theirs`, without a real HTTP layer. */
    function runnerHolds(draft: StoredDraft): void {
      const api = TestBed.inject(ConsoleApi);
      vi.spyOn(api, 'draft').mockReturnValue(of(draft));
    }

    it('adopts a draft when this device has written nothing', () => {
      runnerHolds(theirs);
      drafts.sync('a');
      expect(drafts.incoming()).toEqual({ id: 'a', draft: theirs });
      expect(drafts.clash()).toBeUndefined();
    });

    it('says nothing when the two already agree', () => {
      drafts.put('a', 'from the other device', undefined, { push: false });
      runnerHolds(theirs);
      drafts.sync('a');
      expect(drafts.incoming()).toBeUndefined();
      expect(drafts.clash()).toBeUndefined();
    });

    it('raises a clash rather than overwriting what is here', () => {
      drafts.put('a', 'typed at the desk', undefined, { push: false });
      runnerHolds(theirs);
      drafts.sync('a');
      expect(drafts.clash()).toEqual({ id: 'a', mine: 'typed at the desk', theirs });
      // The local text must survive: replacing it is the silent data loss.
      expect(drafts.text('a')).toBe('typed at the desk');
    });

    it('is quiet when the runner cannot be reached', () => {
      const api = TestBed.inject(ConsoleApi);
      vi.spyOn(api, 'draft').mockReturnValue(throwError(() => new Error('down')));
      drafts.put('a', 'typed at the desk', undefined, { push: false });
      drafts.sync('a');
      expect(drafts.clash()).toBeUndefined();
      expect(drafts.text('a')).toBe('typed at the desk');
    });
  });

  /** Four outcomes, and combining must keep both texts whole. */
  describe('settling a clash', () => {
    const theirs = { text: 'theirs', rev: 4, at: 1000 };

    beforeEach(() => {
      const api = TestBed.inject(ConsoleApi);
      vi.spyOn(api, 'putDraft').mockReturnValue(of({ ...theirs, rev: 5 }));
      drafts.put('a', 'mine', undefined, { push: false });
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
  });

  /**
   * ⚠ **Opening a session is not a statement about its draft.**
   *
   * The composer's recording effect runs once when a session opens, with
   * whatever is on screen — `''` when nothing has been typed here. Pushing that
   * makes a device that has only LOOKED at a conversation a writer of it: it
   * takes revision 1 with an empty text, and the device that actually typed
   * something is then refused and shown a clash against nothing.
   */
  describe('a device that has only looked', () => {
    it('does not push when nothing has changed since the runner was last in step', () => {
      const api = TestBed.inject(ConsoleApi);
      const put = vi.spyOn(api, 'putDraft').mockReturnValue(of({ text: '', rev: 1, at: 1 }));
      vi.useFakeTimers();

      // What the session view does on open with no draft anywhere.
      drafts.put('a', '', undefined);
      vi.runAllTimers();

      expect(put).not.toHaveBeenCalled();
      vi.useRealTimers();
    });

    it('still pushes a draft typed here that the runner has never seen', () => {
      const api = TestBed.inject(ConsoleApi);
      const put = vi.spyOn(api, 'putDraft').mockReturnValue(of({ text: 'x', rev: 1, at: 1 }));
      vi.useFakeTimers();

      drafts.put('a', 'typed while the tunnel was down', undefined);
      vi.runAllTimers();

      expect(put).toHaveBeenCalledOnce();
      vi.useRealTimers();
    });

    it('pushes a deliberate clear, which is how a sent message stops being a draft', () => {
      const api = TestBed.inject(ConsoleApi);
      const put = vi.spyOn(api, 'putDraft').mockReturnValue(of({ text: 'words', rev: 1, at: 1 }));
      vi.useFakeTimers();
      drafts.put('a', 'words', undefined);
      vi.runAllTimers();
      put.mockClear();

      drafts.put('a', '', undefined);
      vi.runAllTimers();

      expect(put).toHaveBeenCalledOnce();
      vi.useRealTimers();
    });
  });

  /**
   * ⚠ **A stale copy is not a competing one.**
   *
   * Pippijn types mostly on one device and switches to the other to paste. The
   * device he switches TO holds the last text it synced, not an empty box — so
   * testing "has this device got text?" calls every handover a conflict. The
   * test that means anything is whether this device has UNSENT changes.
   */
  describe('picking up the other device', () => {
    function runnerHolds(draft: StoredDraft): void {
      vi.spyOn(TestBed.inject(ConsoleApi), 'draft').mockReturnValue(of(draft));
    }

    it('adopts silently when this device has only fallen behind', () => {
      vi.spyOn(TestBed.inject(ConsoleApi), 'putDraft').mockReturnValue(
        of({ text: 'A', rev: 1, at: 1 }),
      );
      vi.useFakeTimers();
      drafts.put('a', 'A', undefined);
      vi.runAllTimers();
      vi.useRealTimers();

      // The other device carried it on.
      runnerHolds({ text: 'A and more', rev: 2, at: 2 });
      drafts.sync('a');

      expect(drafts.clash()).toBeUndefined();
      expect(drafts.incoming()).toEqual({ id: 'a', draft: { text: 'A and more', rev: 2, at: 2 } });
    });

    it('still raises a clash when this device has something unsent', () => {
      vi.spyOn(TestBed.inject(ConsoleApi), 'putDraft').mockReturnValue(
        of({ text: 'A', rev: 1, at: 1 }),
      );
      vi.useFakeTimers();
      drafts.put('a', 'A', undefined);
      vi.runAllTimers();
      vi.useRealTimers();

      // Typed here since, and not yet sent.
      drafts.put('a', 'A plus something of mine', undefined, { push: false });
      runnerHolds({ text: 'A and more', rev: 2, at: 2 });
      drafts.sync('a');

      expect(drafts.clash()?.mine).toBe('A plus something of mine');
      expect(drafts.incoming()).toBeUndefined();
    });
  });

  /**
   * ⚠ **Typing on a train must not cost the words.**
   *
   * The whole reason the local store is what the composer reads. Nothing here
   * may depend on a request succeeding, and a request that failed has to be paid
   * later rather than forgotten — nothing else retries it.
   */
  describe('with no connection at all', () => {
    function offline(): void {
      vi.spyOn(TestBed.inject(ConsoleApi), 'draft').mockReturnValue(
        throwError(() => new Error('no route to host')),
      );
      vi.spyOn(TestBed.inject(ConsoleApi), 'putDraft').mockReturnValue(
        throwError(() => new Error('no route to host')),
      );
    }

    it('keeps every word, and says nothing about the network', () => {
      offline();
      vi.useFakeTimers();
      drafts.put('a', 'written between two stations', undefined);
      vi.runAllTimers();
      vi.useRealTimers();

      expect(drafts.text('a')).toBe('written between two stations');
      expect(drafts.clash()).toBeUndefined();
    });

    it('survives the page being destroyed and rebuilt underground', () => {
      offline();
      drafts.put('a', 'written between two stations', undefined, { push: false });
      TestBed.resetTestingModule();
      expect(TestBed.inject(Drafts).text('a')).toBe('written between two stations');
    });

    it('owes the words, and pays on coming back up', () => {
      offline();
      vi.useFakeTimers();
      drafts.put('a', 'written between two stations', undefined);
      vi.runAllTimers();
      vi.useRealTimers();

      // Above ground: the runner answers, and holds nothing for this session.
      const api = TestBed.inject(ConsoleApi);
      vi.spyOn(api, 'draft').mockReturnValue(of(null));
      const put = vi
        .spyOn(api, 'putDraft')
        .mockReturnValue(of({ text: 'written between two stations', rev: 1, at: 1 }));

      drafts.sync('a');

      expect(put).toHaveBeenCalledWith('a', {
        text: 'written between two stations',
        from: undefined,
      });
    });
  });

  /**
   * ⚠ **The other screen has to change while somebody is looking at it.**
   *
   * Reconciling only on navigation and on returning to the front means two pages
   * open side by side never hear about each other: typing on the phone left the
   * browser stale, and SENDING from the browser left the phone's box holding a
   * message that had already gone. The roster poll carries the draft so that the
   * screen nobody touched is the screen that updates.
   */
  describe('while both screens are open', () => {
    it('shows what the other device typed', () => {
      drafts.reconcile('a', { text: 'typed on the phone', rev: 1, at: 1 });
      expect(drafts.incoming()).toEqual({
        id: 'a',
        draft: { text: 'typed on the phone', rev: 1, at: 1 },
      });
      expect(drafts.clash()).toBeUndefined();
    });

    it('empties when the other device SENDS, because a send clears the draft', () => {
      // This screen is holding what was typed and synced.
      drafts.reconcile('a', { text: 'about to be sent', rev: 1, at: 1 });
      drafts.adopt('a', { text: 'about to be sent', rev: 1, at: 1 });
      expect(drafts.text('a')).toBe('about to be sent');

      // The other screen sends it: the composer empties there, which is an empty
      // write, and arrives here as a tombstone at the next revision.
      drafts.reconcile('a', { text: '', rev: 2, at: 2 });
      expect(drafts.incoming()).toEqual({ id: 'a', draft: { text: '', rev: 2, at: 2 } });
      expect(drafts.clash()).toBeUndefined();
    });

    it('does not send twice when a keystroke is already pending', () => {
      const put = vi
        .spyOn(TestBed.inject(ConsoleApi), 'putDraft')
        .mockReturnValue(of({ text: 'half typed', rev: 1, at: 1 }));
      vi.useFakeTimers();
      drafts.put('a', 'half typed', undefined);
      // The poll lands inside the debounce window.
      drafts.reconcile('a', null);
      vi.runAllTimers();
      expect(put).toHaveBeenCalledOnce();
      vi.useRealTimers();
    });
  });
});
