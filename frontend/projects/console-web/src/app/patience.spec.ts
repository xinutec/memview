import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { Patience } from './patience';

const TROUBLE = { kind: 'runner', what: 'no answer' } as const;

describe('Patience', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('says nothing about trouble that is gone before anyone could read it', () => {
    // The blink this exists to prevent. A burst of polls fails together while the
    // key is renewed, and the browser retries a dropped stream about every three
    // seconds: both are the ORDINARY case, not news.
    const patience = new Patience();
    patience.wrong(TROUBLE);
    vi.advanceTimersByTime(3000);
    patience.right();
    vi.advanceTimersByTime(60_000);
    expect(patience.worth()).toBeUndefined();
  });

  it('says it once the trouble has lasted', () => {
    const patience = new Patience();
    patience.wrong(TROUBLE);
    vi.advanceTimersByTime(8000);
    expect(patience.worth()).toEqual(TROUBLE);
  });

  it('does not restart the clock each time the trouble recurs', () => {
    // ⚠ **Where a naive version never fires.** A poll every five seconds against a
    // runner that is down is trouble reported every five seconds; restarting on
    // each one leaves a permanently broken console permanently silent.
    const patience = new Patience();
    patience.wrong(TROUBLE);
    vi.advanceTimersByTime(5000);
    patience.wrong(TROUBLE);
    vi.advanceTimersByTime(3000);
    expect(patience.worth()).toEqual(TROUBLE);
  });

  it('shows the latest word on the trouble, not the first', () => {
    const patience = new Patience();
    patience.wrong(TROUBLE);
    patience.wrong({ kind: 'runner', what: 'the runner answered 503' });
    vi.advanceTimersByTime(8000);
    expect(patience.worth()).toEqual({ kind: 'runner', what: 'the runner answered 503' });
  });

  it('clears what is on screen the moment things are well', () => {
    const patience = new Patience();
    patience.wrong(TROUBLE);
    vi.advanceTimersByTime(8000);
    patience.right();
    expect(patience.worth()).toBeUndefined();
  });

  it('knows trouble is under way before it is worth saying', () => {
    // What decides whether a kept copy is worth offering — that question wants the
    // raw state, not the patient one.
    const patience = new Patience();
    patience.wrong(TROUBLE);
    expect(patience.troubled).toBe(true);
    expect(patience.worth()).toBeUndefined();
  });
});
