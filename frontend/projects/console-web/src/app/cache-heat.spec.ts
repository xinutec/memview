import { describe, expect, it } from 'vitest';

import { CacheHeat, withinCacheHour } from './cache-heat';

const heat = (at: number | undefined, now?: number): number => new CacheHeat().transform(at, now);

const NOW = 1_700_000_000_000;
const at = (minutes: number): number => NOW - minutes * 60_000;

describe('withinCacheHour', () => {
  it('stops at the hour, because that is when the decision is gone', () => {
    // ⚠ Not clamped-and-shown: a red clock on a session whose cache went hours
    // ago warns about something nobody can still act on, and would sit red on
    // every idle row until it read as decoration.
    expect(withinCacheHour(at(59), NOW)).toBe(true);
    expect(withinCacheHour(at(60), NOW)).toBe(false);
    expect(withinCacheHour(at(8 * 60), NOW)).toBe(false);
  });

  it('has nothing to show for a missing timestamp', () => {
    expect(withinCacheHour(undefined, NOW)).toBe(false);
  });
});

describe('CacheHeat', () => {
  it('stays quiet through the first half', () => {
    // 20-30 minutes is not yet a worry, so the colour must barely have moved.
    expect(heat(at(0), NOW)).toBe(0);
    expect(heat(at(20), NOW)).toBeLessThan(0.05);
    expect(heat(at(30), NOW)).toBeLessThan(0.15);
  });

  it('is clearly moving by 40-50, where the session has to be steered', () => {
    expect(heat(at(40), NOW)).toBeGreaterThan(0.25);
    expect(heat(at(50), NOW)).toBeGreaterThan(0.55);
  });

  it('is loud through the last ten minutes', () => {
    expect(heat(at(55), NOW)).toBeGreaterThan(0.75);
    expect(heat(at(59), NOW)).toBeGreaterThan(0.9);
  });

  it('accelerates rather than ramping evenly', () => {
    // ⚠ The property that makes it useful, stated as a property: the second half
    // must gain far more than the first, or 50-59 reads like 10-19.
    const firstHalf = heat(at(30), NOW) - heat(at(0), NOW);
    const secondHalf = heat(at(59), NOW) - heat(at(30), NOW);
    expect(secondHalf).toBeGreaterThan(firstHalf * 5);
  });

  it('reads a future timestamp as cold, not as negative', () => {
    expect(heat(NOW + 60_000, NOW)).toBe(0);
  });
});
