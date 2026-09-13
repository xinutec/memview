import { Pipe, PipeTransform } from '@angular/core';

/** The prompt cache's life, in ms. Past this a turn re-reads the whole context. */
export const CACHE_TTL_MS = 60 * 60 * 1000;

/**
 * Shapes the alarm against when the reader needs to ACT, not against the clock.
 *
 * ⚠ **Cubed, and linear was wrong.** The first version ramped evenly on the
 * argument that a cache expires rather than decaying, so distance to the
 * deadline is the honest signal. That describes the cache; it does not describe
 * the decision. Nothing is owed at 20-30 minutes, 40-50 is when a session has to
 * be steered towards an ending, and 50-59 is urgent — so the colour has to stay
 * quiet through the first half and move fast through the last ten minutes.
 *
 *     10m 0.005    30m 0.13    45m 0.42    52m 0.65    57m 0.86    59m 0.95
 */
const URGENCY = 3;

/** Whether the hour is still running, and so whether there is anything to show. */
export function withinCacheHour(at: number | undefined, now: number = Date.now()): boolean {
  if (at === undefined) return false;
  return now - at < CACHE_TTL_MS;
}

/**
 * How urgent this session's remaining cache is, 0 to 1.
 *
 * Drives a colour, not a number: the reader wants to notice the approach without
 * reading a clock.
 *
 * ⚠ **Past the hour there is nothing to report and the icon is not drawn at all**
 * — see [[withinCacheHour]]. A permanently red clock on a session whose cache
 * went hours ago is a warning about a decision nobody can still make, and it
 * would sit red on every idle row until it read as decoration.
 */
@Pipe({ name: 'cacheHeat' })
export class CacheHeat implements PipeTransform {
  transform(at: number | undefined, now: number = Date.now()): number {
    if (at === undefined) return 0;
    // A timestamp in the future is a clock disagreement between the host writing
    // the transcript and the browser reading it, not a negative age.
    const through = Math.min(1, Math.max(0, (now - at) / CACHE_TTL_MS));
    return through ** URGENCY;
  }
}
