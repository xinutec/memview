/**
 * Whether the runner being unreachable is worth saying yet. Most failed polls
 * are gone within a second — a burst failing together, then `host.ts` renews —
 * so trouble must outlive one poll. Every episode over a few minutes still
 * shows. For the POLL only: an action that fails says so on the press.
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
   * Takes the composed sentence, so the caller keeps the wording.
   */
  failed(message: string): string {
    this.missed += 1;
    return this.missed >= Reach.PATIENCE ? message : '';
  }
}
