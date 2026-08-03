import { TestBed } from '@angular/core/testing';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { Updates } from './updates';

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
    expect(updates.waiting).toBe(true);
  });

  it('takes the held reload the moment the app is put away', () => {
    updates.saw('aaaa');
    vi.setSystemTime(Date.now() + 60_000);
    updates.saw('bbbb');
    visibility('hidden');
    expect(reload).toHaveBeenCalledOnce();
  });

  it('reloads straight away when the update lands while hidden', () => {
    updates.saw('aaaa');
    vi.setSystemTime(Date.now() + 60_000);
    visibility('hidden');
    updates.saw('bbbb');
    expect(reload).toHaveBeenCalledOnce();
  });
});
