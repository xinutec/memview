/** How long before the login lapses the console starts saying so. */
export const WARN_MS = 3 * 24 * 3600_000;

/** What to say about the login's lapse, or nothing while it is far off. */
export interface LapseWarning {
  /** Past it: every session is refused until somebody signs in. */
  readonly lapsed: boolean;
  readonly lead: string;
  readonly what: string;
}

/**
 * The warning for a login lapsing at `lapses` (epoch ms), seen at `now`. Signing
 * in again resets the lapse to about four weeks out, so the warning goes.
 */
export function lapseWarning(lapses: number | undefined, now: number): LapseWarning | undefined {
  if (lapses === undefined) return undefined;
  const left = lapses - now;
  if (left <= 0) {
    return {
      lapsed: true,
      lead: 'Signed out.',
      what: "Claude's login has lapsed; every session needs it.",
    };
  }
  if (left > WARN_MS) return undefined;
  return {
    lapsed: false,
    lead: 'Sign in soon.',
    what: `Claude's login lapses ${within(left)}; signing in again resets it.`,
  };
}

/** A time ahead, in the largest whole unit: "in 2 days", "in 5 hours". */
function within(ms: number): string {
  const hours = Math.floor(ms / 3600_000);
  if (hours >= 48) return `in ${Math.floor(hours / 24)} days`;
  if (hours >= 1) return `in ${hours} hour${hours === 1 ? '' : 's'}`;
  return 'within the hour';
}
