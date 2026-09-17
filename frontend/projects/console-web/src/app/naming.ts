/**
 * What to call a conversation on screen. One rule in one place, because the
 * list and the session page disagreed once, and a tap that lands on a different
 * name reads as having opened the wrong thing.
 */

/** The folder a session runs in, as the one word anybody uses for it. */
export function placeOf(dir: string): string {
  return dir.split('/').filter(Boolean).pop() ?? dir;
}

/**
 * A session's name, or something that identifies it when it has not taken one.
 * The name arrives late — the runner reads it out of the transcript — so the
 * fallback is what every new session shows first. The folder alone identifies
 * nothing (every session starts in `~/Code`), so the short id disambiguates.
 */
export function titleOf(session: { name?: string; dir: string; id?: string }): string {
  if (session.name) return session.name;
  const place = placeOf(session.dir);
  return session.id ? `${place} · ${session.id.slice(0, 8)}` : place;
}
