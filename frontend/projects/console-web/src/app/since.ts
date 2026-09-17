/**
 * How long ago something happened, to the coarsest unit that still says
 * something. Shared, so three screens agree about whether 90 seconds is `1m`.
 */
export function since(at: number, now: number = Date.now()): string {
  const minutes = Math.max(0, (now - at) / 60000);
  if (minutes < 1) return 'just now';
  if (minutes < 60) return `${Math.round(minutes)}m ago`;
  if (minutes < 60 * 24) return `${Math.round(minutes / 60)}h ago`;
  return `${Math.round(minutes / 1440)}d ago`;
}
