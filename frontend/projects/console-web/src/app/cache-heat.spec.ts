import { describe, expect, it } from 'vitest';

import { CACHE_TTL_MS, CacheHeat } from './cache-heat';

const heat = (at: number | undefined, now?: number): number => new CacheHeat().transform(at, now);

const NOW = 1_700_000_000_000;

describe('CacheHeat', () => {
  it('is cold at the start of the hour and hot at its end', () => {
    expect(heat(NOW, NOW)).toBe(0);
    expect(heat(NOW - CACHE_TTL_MS, NOW)).toBe(1);
  });

  it('runs linearly between them', () => {
    // Linear because the cache expires rather than decaying: what is being
    // reported is distance to a deadline.
    expect(heat(NOW - CACHE_TTL_MS / 2, NOW)).toBeCloseTo(0.5);
    expect(heat(NOW - CACHE_TTL_MS / 4, NOW)).toBeCloseTo(0.25);
  });

  it('stops at 1 rather than running on', () => {
    // ⚠ An 8-hour-old session is not 8x more urgent than a 1-hour-old one: both
    // have lost the cache. A colour that kept moving would report a difference
    // that does not exist.
    expect(heat(NOW - 8 * CACHE_TTL_MS, NOW)).toBe(1);
  });

  it('reads a future timestamp as cold, not as negative', () => {
    // Clock disagreement between the host writing the transcript and the browser
    // reading it. Negative heat would render as an out-of-range colour.
    expect(heat(NOW + 60_000, NOW)).toBe(0);
  });

  it('has nothing to say about a missing timestamp', () => {
    expect(heat(undefined, NOW)).toBe(0);
  });
});
