/**
 * How long ago something happened, to the coarsest unit that still says
 * something.
 *
 * Shared because three screens now ask the same question — the roster's rows,
 * the usage strip and a draft clash — and three answers that disagree about
 * whether 90 seconds is `1m` or `2m` read as a bug in whichever one you are
 * looking at second.
 */
export function since(at: number, now: number = Date.now()): string {
  const minutes = Math.max(0, (now - at) / 60000);
  if (minutes < 1) return 'just now';
  if (minutes < 60) return `${Math.round(minutes)}m ago`;
  if (minutes < 60 * 24) return `${Math.round(minutes / 60)}h ago`;
  return `${Math.round(minutes / 1440)}d ago`;
}
