/**
/**
 * Whether the runner being unreachable is worth saying yet.
 *
 * ⚠ **A failed poll is usually gone before anybody could read the banner.** In
 * this console's own telemetry most failures fall in episodes lasting under a
 * second — a burst of in-flight requests failing together, then `host.ts` renews
 * and the next poll succeeds. Raising it on the first failure put "cannot reach
 * the runner" on screen repeatedly for something already fixed.
 *
 * Requiring trouble to outlive one poll removes most of them, and raising the
 * threshold further barely moves the count: these are two populations rather
 * than a spread. **Every episode over a few minutes still shows**, which is the
 * property that matters — this hides blips, not failures.
 *
 * ⚠ **For the POLL only.** An action that fails must say so on the press,
 * because somebody is waiting to learn whether it landed; those go through
 * `trouble` and report at once. Same wording, different patience.
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
