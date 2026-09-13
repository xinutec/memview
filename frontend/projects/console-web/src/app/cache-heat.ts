import { Pipe, PipeTransform } from '@angular/core';

/** The prompt cache's life, in ms. Past this a turn re-reads the whole context. */
export const CACHE_TTL_MS = 60 * 60 * 1000;

/**
 * How far a session is through the hour its prompt cache lasts, 0 to 1.
 *
 * Drives a colour, not a number: at 0 there is nothing to think about, and by 1
 * the cache is gone. What the reader wants is to notice the approach without
 * reading a clock, so this is fed to a hue rather than printed.
 *
 * ⚠ **Clamped at 1, and it stays there.** An eight-hour-old session is not
 * eight times as urgent as a one-hour-old one — both have lost the cache, and
 * the question is already answered. Letting it run past 1 would make the colour
 * keep changing after the only thing it reports has stopped being true.
 *
 * ⚠ **Linear, deliberately.** An ease would be prettier and would lie: the cache
 * does not decay, it expires, so the honest signal is distance to a deadline.
 */
@Pipe({ name: 'cacheHeat' })
export class CacheHeat implements PipeTransform {
  transform(at: number | undefined, now: number = Date.now()): number {
    if (at === undefined) return 0;
    // A timestamp in the future is a clock disagreement, not a fresh session
    // with negative age; 0 is the honest reading of "nothing to worry about".
    return Math.min(1, Math.max(0, (now - at) / CACHE_TTL_MS));
  }
}
