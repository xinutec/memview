import { Injectable, signal } from '@angular/core';

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
 * - **Restored by going back**: a boot, because that is what it is — see the
 *   `pageshow` listener. Startup then decides, so a page that is not behind is
 *   left alone.
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
  private pending = signal(false);

  constructor() {
    document.addEventListener('visibilitychange', () => {
      if (document.visibilityState === 'hidden' && this.pending()) this.reload();
    });
    // ⚠ **A restored page is a boot, not a session being sat on.** A reload
    // replaces the CURRENT history entry's document; every entry made before it
    // still belongs to the old one, which the browser keeps alive — so going back
    // resurrects a live old bundle, and this console pushes an entry per sheet,
    // so there is usually one to land on. The mid-session rule would then hold
    // the reload, which is right for a page with a half-typed instruction in it
    // and wrong for one just navigated into. Resetting the clock reuses the
    // decision already here: stale reloads at once, current does nothing at all.
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
   * Whether a newer bundle is waiting for the app to be put away.
   *
   * Rendered, because a held reload is otherwise indistinguishable from no
   * update at all — somebody waiting to see the page refresh would wait for
   * ever, correctly, and have no way to tell that from a broken check.
   */
  readonly waiting = this.pending.asReadonly();

  /** Its own method so a test can assert the decision without navigating. */
  reload(): void {
    document.location.reload();
  }
}
