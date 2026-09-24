import { Injectable, signal } from '@angular/core';

/**
 * Reload when the bundle underneath the page changes. No service worker, so the
 * fingerprint the runner reports in `/api/state` is the only signal: the first
 * one seen is what this page booted from.
 *
 * The timing is life's, adapted, so a half-typed instruction is never thrown
 * away: at startup or hidden, reload at once; visible mid-session, hold it and
 * reload when the app is next put away; restored by going back, a boot. No
 * reload loop: after reloading, the fingerprint served is the one booted from.
 */
@Injectable({ providedIn: 'root' })
export class Updates {
  /** Updates arriving this soon after boot reload immediately. */
  private static readonly STARTUP_MS = 10_000;

  private booted = Date.now();
  private serving?: string;
  private pending = signal(false);

  constructor() {
    document.addEventListener('visibilitychange', () => {
      if (document.visibilityState === 'hidden' && this.pending()) this.reload();
    });
    // A restored page is a boot: a reload replaces the current history entry's
    // document, and going back resurrects a live old bundle — this console pushes
    // an entry per sheet, so there is usually one to land on.
    window.addEventListener('pageshow', (event: PageTransitionEvent) => {
      if (event.persisted) this.booted = Date.now();
    });
  }

  /** Called with every state response. */
  saw(bundle: string | undefined): void {
    if (!bundle) return;
    this.serving ??= bundle;
    if (bundle === this.serving) return;
    const starting = Date.now() - this.booted < Updates.STARTUP_MS;
    if (starting || document.visibilityState === 'hidden') this.reload();
    else this.pending.set(true);
  }

  /**
   * Whether a newer bundle is waiting for the app to be put away. Rendered,
   * since a held reload is otherwise indistinguishable from no update.
   */
  readonly waiting = this.pending.asReadonly();

  /** Its own method so a test can assert the decision without navigating. */
  reload(): void {
    document.location.reload();
  }
}
