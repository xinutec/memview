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
 * A session's name, or something that identifies it when it has not taken one.
 *
 * ⚠ **The name arrives late.** The runner reads it out of the transcript, so a
 * session started a second ago has none — the fallback is the first thing every
 * new session shows, not an edge case.
 *
 * ⚠ **The folder alone identifies nothing.** Every session here is started in
 * `~/Code`, so the folder is `Code` for all of them. The short id disambiguates,
 * and is also how a session is addressed in the tasks service. It is the CLI's
 * own last resort for the same question.
 *
 * One rule taking one session and nothing else: the list, the toolbar and the
 * details sheet all call this, they disagreed once already, and a rule needing
 * data only one of them has will disagree again.
 */
export function titleOf(session: { name?: string; dir: string; id?: string }): string {
  if (session.name) return session.name;
  const place = placeOf(session.dir);
  return session.id ? `${place} · ${session.id.slice(0, 8)}` : place;
}
