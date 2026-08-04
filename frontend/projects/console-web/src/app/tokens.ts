/**
 * How full a conversation's context is, in the words a glance wants.
 *
 * Shared by the list and the session it opens, because it is one fact about one
 * conversation and it should not read as two: `496k / 1M` in the header over a
 * row that said `496,231` would look like two measurements of different things.
 */

/** Tokens, at the precision a glance wants: `496k`, `1M`. */
export function tokens(count: number): string {
  if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(count % 1_000_000 === 0 ? 0 : 1)}M`;
  return `${Math.round(count / 1000)}k`;
}

/**
 * How full the context is, as `496k / 1M`, when there is anything to say.
 *
 * ⚠ **The window is declared on the result line and nowhere else** — not in the
 * transcript, so a conversation that is not running, and a resumed one that has
 * not finished a turn, know how full they are and not what they are full of.
 * The count alone beats showing nothing: the number people watch for is the
 * first one.
 */
export function fullness(context?: number, window?: number): string | undefined {
  if (!context) return undefined;
  return window ? `${tokens(context)} / ${tokens(window)}` : tokens(context);
}
