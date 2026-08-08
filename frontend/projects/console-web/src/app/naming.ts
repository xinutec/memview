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
 * ⚠ **The name arrives late.** It is read out of the transcript by the runner —
 * a `custom-title` or `agent-name` line, written by `/rename` or by enrolment —
 * so a session started a second ago has none. The fallback is not an edge case
 * but the first thing every new session shows.
 *
 * ⚠ **And the folder alone stopped identifying anything.** Every session on this
 * machine is started in `~/Code`, the parent of every repository, because that
 * is how they are worked with — so the folder is `Code` for all of them and two
 * new sessions were indistinguishable on the list. The short id disambiguates,
 * and it is also the thing needed to claim a task list
 * (`claude_tasks.py --session <id>`), so it is worth having on screen.
 *
 * The id is the CLI's own last resort for the same question — its session
 * labeller ends `… || sessionId.slice(0, 8)`, read off the 2.1.221 binary; see
 * `reader/src/transcript.rs`.
 *
 * One rule taking one session and nothing else, deliberately: the list, the
 * toolbar and the details sheet all call this, they disagreed once already, and
 * a rule needing data only one of them has is a rule that will disagree again.
 */
export function titleOf(session: { name?: string; dir: string; id?: string }): string {
  if (session.name) return session.name;
  const place = placeOf(session.dir);
  return session.id ? `${place} · ${session.id.slice(0, 8)}` : place;
}
