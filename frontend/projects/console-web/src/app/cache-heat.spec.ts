import { describe, expect, it } from 'vitest';

import { cacheStops, cacheUrgent, withinCacheHour } from './cache-heat';

const NOW = 1_700_000_000_000;
const at = (minutes: number): number => NOW - minutes * 60_000;

describe('withinCacheHour', () => {
  it('stops at the hour, because that is when the decision is gone', () => {
    // Not clamped-and-shown: a red clock on a session whose cache went hours
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

describe('cacheStops', () => {
  it('lands the three colours on the minutes they are named for', () => {
    // The whole point of the two legs. Yellow is complete at 20 and the
    // yellow-to-red sweep is half done at 40, which is where orange is.
    expect(cacheStops(at(20), NOW)).toEqual({ warm: 1, hot: 0 });
    expect(cacheStops(at(40), NOW).hot).toBeCloseTo(0.5);
    expect(cacheStops(at(60), NOW)).toEqual({ warm: 1, hot: 1 });
  });

  it('is already arriving in the first twenty minutes', () => {
    // Seeing something at 20-30 is not a worry; seeing nothing until 40 is.
    expect(cacheStops(at(10), NOW).warm).toBeCloseTo(0.5);
  });

  it('spends each leg evenly, because the hue does the escalating', () => {
    const hot = (m: number): number => cacheStops(at(m), NOW).hot;
    expect(hot(50) - hot(40)).toBeCloseTo(hot(60) - hot(50));
  });

  it('starts cold and reads a future timestamp the same way', () => {
    expect(cacheStops(at(0), NOW)).toEqual({ warm: 0, hot: 0 });
    expect(cacheStops(NOW + 60_000, NOW)).toEqual({ warm: 0, hot: 0 });
    expect(cacheStops(undefined, NOW)).toEqual({ warm: 0, hot: 0 });
  });
});

describe('cacheUrgent', () => {
  it('turns on for the last ten minutes only', () => {
    expect(cacheUrgent(at(49), NOW)).toBe(false);
    expect(cacheUrgent(at(50), NOW)).toBe(true);
    expect(cacheUrgent(undefined, NOW)).toBe(false);
  });
});
