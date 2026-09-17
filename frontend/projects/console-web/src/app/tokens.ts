/**
 * How full a conversation's context is, in the words a glance wants. Shared by
 * the list and the session, so `496k / 1M` and `496,231` do not read as two
 * measurements.
 */

/** Tokens, at the precision a glance wants: `496k`, `1M`. */
export function tokens(count: number): string {
  if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(count % 1_000_000 === 0 ? 0 : 1)}M`;
  return `${Math.round(count / 1000)}k`;
}

/**
 * How full the context is, as `496k / 1M`, when there is anything to say. The
 * window is declared on the result line only, so a conversation that is not
 * running knows how full it is and not what it is full of; the count alone
 * beats nothing.
 */
export function fullness(context?: number, window?: number): string | undefined {
  if (!context) return undefined;
  return window ? `${tokens(context)} / ${tokens(window)}` : tokens(context);
}
