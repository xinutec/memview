/**
 * Whether the runner being unreachable is worth saying yet.
 *
 * ⚠ **A failed poll is usually gone before anybody could read the banner.**
 * Measured over 3.9 days of this console's own client telemetry — 17,307 events,
 * of which 2,843 were status 0 in 235 episodes — the median episode is under a
 * second: a burst of in-flight requests failing together, then `host.ts` fires
 * `consoleHost.renew()` and the next poll succeeds. Raising the banner on the
 * first failure put "cannot reach the runner" on screen every 24 minutes for
 * something that had already fixed itself.
 *
 * So the rule is that trouble must outlive one poll. Requiring that:
 *
 * ```text
 * outlives  5s (one poll):  81 of 238 episodes still shown — one every 1.2h
 * outlives 10s          :  80 of 238
 * outlives 20s          :  78 of 238
 * ```
 *
 * A cliff at the first poll interval and nothing after it, so the distribution
 * is two populations rather than a spread: sub-poll bursts, and real outages.
 * **All 26 episodes longer than five minutes still show**, which is the property
 * that matters — this hides blips, not failures.
 *
 * ⚠ **For the POLL only.** A user-initiated action that fails must say so on the
 * press, because the person is waiting to learn whether it landed; those go
 * through `trouble` and report immediately. That split is why this is a separate
 * object rather than a rule inside [[reason]] — the wording is the same, the
 * patience is not.
 *
 * Kept here rather than copied into the two views that poll, so the threshold is
 * stated once.
 */
export class Reach {
  /** Consecutive failed polls before the trouble is worth showing. */
  static readonly PATIENCE = 2;

  private missed = 0;

  /** A poll answered. Nothing is wrong, and anything shown should go. */
  answered(): string {
    this.missed = 0;
    return '';
  }

  /**
   * A poll failed. The message to show, or empty while this may still be a blip.
   *
   * Takes the composed sentence rather than the error, so the caller keeps
   * ownership of the wording and this owns only the patience.
   */
  failed(message: string): string {
    this.missed += 1;
    return this.missed >= Reach.PATIENCE ? message : '';
  }
}
