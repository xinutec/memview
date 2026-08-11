import { TestBed } from '@angular/core/testing';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { Updates } from './updates';

/**
 * A `pageshow`, as the browser fires it: `persisted` is true only when this
 * document came back out of the back/forward cache rather than being loaded.
 */
function restored(persisted: boolean): void {
  window.dispatchEvent(new PageTransitionEvent('pageshow', { persisted }));
}

/** Pretend the page is visible or not, and let the listener hear about it. */
function visibility(state: 'visible' | 'hidden'): void {
  Object.defineProperty(document, 'visibilityState', { value: state, configurable: true });
  document.dispatchEvent(new Event('visibilitychange'));
}

describe('Updates', () => {
  let updates: Updates;
  let reload: ReturnType<typeof vi.fn<() => void>>;

  beforeEach(() => {
    visibility('visible');
    updates = TestBed.inject(Updates);
    reload = vi.fn<() => void>();
    updates.reload = reload;
  });

  it('does nothing while the bundle it booted from is the one being served', () => {
    updates.saw('aaaa');
    updates.saw('aaaa');
    expect(reload).not.toHaveBeenCalled();
  });

  it('ignores a runner that serves no bundle at all', () => {
    // The desk runs `ng serve`, so the runner has no bundle to fingerprint. That
    // is a normal configuration, not a reason to reload.
    updates.saw(undefined);
    updates.saw(undefined);
    expect(reload).not.toHaveBeenCalled();
  });

  it('reloads at once when a new bundle lands during startup', () => {
    // Nothing is in progress in the first seconds, so the reload is invisible.
    updates.saw('aaaa');
    updates.saw('bbbb');
    expect(reload).toHaveBeenCalledOnce();
  });

  it('holds the reload while somebody is looking at the page', () => {
    // The case this exists for: a half-typed instruction, on a phone, must not
    // be thrown away because a build landed.
    updates.saw('aaaa');
    vi.setSystemTime(Date.now() + 60_000);
    updates.saw('bbbb');
    expect(reload).not.toHaveBeenCalled();
    expect(updates.waiting()).toBe(true);
  });

  it('takes the held reload the moment the app is put away', () => {
    updates.saw('aaaa');
    vi.setSystemTime(Date.now() + 60_000);
    updates.saw('bbbb');
    visibility('hidden');
    expect(reload).toHaveBeenCalledOnce();
  });

  it('treats a page restored by going back as a boot', () => {
    // A left-swipe back lands on a history entry belonging to the PREVIOUS
    // document, which the browser kept alive — not stale HTML, a live old
    // bundle. Reported from the phone 2026-08-07 straight after a deploy.
    updates.saw('aaaa');
    vi.setSystemTime(Date.now() + 60_000);
    restored(true);
    updates.saw('bbbb');
    expect(reload).toHaveBeenCalledOnce();
  });

  it('leaves a restored page alone when it is not behind', () => {
    // Going back is not itself a reason to reload. Most of the time the page
    // that was restored is the current build, and a reload would be a visible
    // flash bought for nothing.
    updates.saw('aaaa');
    vi.setSystemTime(Date.now() + 60_000);
    restored(true);
    updates.saw('aaaa');
    expect(reload).not.toHaveBeenCalled();
  });

  it('does not take a first load as a reason to forget how old the page is', () => {
    // `pageshow` also fires on an ordinary load, with persisted false. Treating
    // that as a restore would reset the clock on a page somebody is sitting on,
    // and the held reload is what protects a half-typed instruction.
    updates.saw('aaaa');
    vi.setSystemTime(Date.now() + 60_000);
    restored(false);
    updates.saw('bbbb');
    expect(reload).not.toHaveBeenCalled();
    expect(updates.waiting()).toBe(true);
  });

  it('reloads straight away when the update lands while hidden', () => {
    updates.saw('aaaa');
    vi.setSystemTime(Date.now() + 60_000);
    visibility('hidden');
    updates.saw('bbbb');
    expect(reload).toHaveBeenCalledOnce();
  });
});
