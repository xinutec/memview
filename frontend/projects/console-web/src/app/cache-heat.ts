/** The prompt cache's life, in ms. Past this a turn re-reads the whole context. */
export const CACHE_TTL_MS = 60 * 60 * 1000;

/** Where the ramp reaches yellow. Before it, the colour is only arriving. */
const YELLOW_MIN = 20;

/** Where the last band starts, and the weight takes over from the hue. */
const URGENT_MIN = 50;

/** Whether the hour is still running, and so whether there is anything to show. */
export function withinCacheHour(at: number | undefined, now: number = Date.now()): boolean {
  if (at === undefined) return false;
  return now - at < CACHE_TTL_MS;
}

/** How far along each leg of the ramp, 0 to 1 — read as a colour, not a number. */
export interface CacheStops {
  /** Neutral to yellow, over the first 20 minutes. */
  warm: number;
  /** Yellow to red, over the remaining 40. */
  hot: number;
}

/**
 * Two straight legs, not a curve: yellow at 20 minutes says something is there
 * without saying anything is wrong, and orange falls out of the sweep at 40.
 */
export function cacheStops(at: number | undefined, now: number = Date.now()): CacheStops {
  if (at === undefined) return { warm: 0, hot: 0 };
  // A future timestamp is the host's clock disagreeing with the browser's.
  const minutes = Math.max(0, (now - at) / 60_000);
  const leg = (from: number, span: number): number =>
    Math.min(1, Math.max(0, (minutes - from) / span));
  return { warm: leg(0, YELLOW_MIN), hot: leg(YELLOW_MIN, 60 - YELLOW_MIN) };
}

/** Whether this is the band that has to be seen rather than noticed. */
export function cacheUrgent(at: number | undefined, now: number = Date.now()): boolean {
  if (at === undefined) return false;
  return (now - at) / 60_000 >= URGENT_MIN;
}
