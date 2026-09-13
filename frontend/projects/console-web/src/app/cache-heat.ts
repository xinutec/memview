import { Pipe, PipeTransform } from '@angular/core';

/** The prompt cache's life, in ms. Past this a turn re-reads the whole context. */
export const CACHE_TTL_MS = 60 * 60 * 1000;

/**
 * ⚠ **Cubed, not linear.** Distance to the deadline describes the cache; it does
 * not describe the decision. Nothing is owed at 20-30 minutes,
 * 40-50 is when a session has to be steered towards an ending, and 50-59 is
 * urgent.
 *
 *     10m 0.005    30m 0.13    45m 0.42    52m 0.65    57m 0.86    59m 0.95
 */
const URGENCY = 3;

/** Whether the hour is still running, and so whether there is anything to show. */
export function withinCacheHour(at: number | undefined, now: number = Date.now()): boolean {
  if (at === undefined) return false;
  return now - at < CACHE_TTL_MS;
}

/** How urgent the remaining cache is, 0 to 1 — read as a colour, not a number. */
@Pipe({ name: 'cacheHeat' })
export class CacheHeat implements PipeTransform {
  transform(at: number | undefined, now: number = Date.now()): number {
    if (at === undefined) return 0;
    // A future timestamp is the host's clock disagreeing with the browser's, not
    // a negative age.
    const through = Math.min(1, Math.max(0, (now - at) / CACHE_TTL_MS));
    return through ** URGENCY;
  }
}
