/**
 * What to call a conversation on screen.
 *
 * One rule, in one place, because it is asked in two: the list titles every card
 * with it and the session page titles itself with it. They disagreed once — the
 * card said `health` and the page it opened said
 * `/home/example/Code/health/packages/health-sync-backend/src/decode` — and a
 * tap that lands somewhere with a different name reads as having opened the
 * wrong thing.
 */

/** The folder a session runs in, as the one word anybody uses for it. */
export function placeOf(dir: string): string {
  return dir.split('/').filter(Boolean).pop() ?? dir;
}

/**
 * A session's name, or where it is running when it has not taken one.
 *
 * ⚠ **The name arrives late.** It is read out of the transcript by the runner,
 * so a session started a second ago has none — the fallback is not an edge case
 * but the first thing every new session shows.
 */
export function titleOf(session: { name?: string; dir: string }): string {
  return session.name ?? placeOf(session.dir);
}
