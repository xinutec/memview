import { Injectable } from '@angular/core';

/**
 * Reload when the bundle underneath the page changes.
 *
 * The console has no service worker, so nothing tells a long-lived page that a
 * new build has been written — you reload by hand or you keep running the old
 * app, and the second one is silent. It does poll `/api/state` already, and the
 * runner now reports a fingerprint of the bundle it is serving, so the check is
 * free: the first fingerprint seen is what this page booted from, and any later
 * one means the page is stale.
 *
 * ⚠ **The timing is the whole design, and it is life's, adapted.** Reloading the
 * instant a build lands would throw away a half-typed instruction — on a phone,
 * where retyping it is the expensive part. So:
 *
 * - **Startup**: reload at once. Nothing is in progress, and you never see it.
 * - **Hidden**: reload at once. Nobody is looking.
 * - **Visible, mid-session**: hold it, and reload when the app is next put away.
 *   A console left open for a day is fresh the next time you pick it up.
 *
 * There is no reload loop: after reloading, the fingerprint the page booted from
 * is the one being served, so nothing more happens.
 */
@Injectable({ providedIn: 'root' })
export class Updates {
  /** Updates arriving this soon after boot reload immediately. */
  private static readonly STARTUP_MS = 10_000;

  private booted = Date.now();
  private serving?: string;
  private pending = false;

  constructor() {
    document.addEventListener('visibilitychange', () => {
      if (document.visibilityState === 'hidden' && this.pending) this.reload();
    });
  }

  /** Called with every state response. */
  saw(bundle: string | undefined): void {
    if (!bundle) return;
    this.serving ??= bundle;
    if (bundle === this.serving) return;
    const starting = Date.now() - this.booted < Updates.STARTUP_MS;
    if (starting || document.visibilityState === 'hidden') this.reload();
    else this.pending = true;
  }

  /** Whether a newer bundle is waiting for the app to be put away. */
  get waiting(): boolean {
    return this.pending;
  }

  /** Its own method so a test can assert the decision without navigating. */
  reload(): void {
    document.location.reload();
  }
}
