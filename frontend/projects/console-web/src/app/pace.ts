/**
 * Whether a window's spend is keeping time with the window.
 *
 * - `even`: on course, or too early to say.
 * - `spare`: at this pace a share of the allowance goes unused, to be burnt late at night.
 * - `over`: at this pace some of the usual rest of the window will not fit.
 * - `short`: at least half of it will not.
 */
export type Pace = 'even' | 'spare' | 'over' | 'short';

/** Local hours in which the allowance is meant to be spent. */
const WAKE = 8;
const SLEEP = 22;

/** Before this share of the window, one heavy stretch projects to anything. */
const GRACE = 1 / 7;

/** Share of the rest of the window that would not fit: amber, then red. */
const OVER = 0.15;
const SHORT = 0.5;

/** Below this much time without allowance, a shortfall is none: the tail of any window is short. */
const LEAST_LOST_MS = 30 * 60_000;

/** Points of allowance left unused at the reset. */
const SPARE = 15;

/**
 * `pct` spent with `through` (0–1) of the window gone and `ahead` ms of it left.
 * The pace so far is carried over the rest, which is how the median of past
 * weeks actually filled.
 */
export function pace(pct: number, through: number, ahead: number): Pace {
  if (through < GRACE || through >= 1 || pct <= 0) return 'even';
  const rest = (pct * (1 - through)) / through;
  const room = 100 - pct;
  if (rest > room) {
    const lost = 1 - room / rest;
    if (lost * ahead < LEAST_LOST_MS) return 'even';
    if (lost >= SHORT) return 'short';
    return lost >= OVER ? 'over' : 'even';
  }
  return room - rest >= SPARE ? 'spare' : 'even';
}

/** Waking milliseconds between two instants, in this device's time zone. */
export function awake(from: number, to: number): number {
  let total = 0;
  const day = new Date(from);
  day.setHours(0, 0, 0, 0);
  while (day.getTime() < to) {
    const up = new Date(day).setHours(WAKE);
    const down = new Date(day).setHours(SLEEP);
    total += Math.max(0, Math.min(down, to) - Math.max(up, from));
    day.setDate(day.getDate() + 1);
  }
  return total;
}
