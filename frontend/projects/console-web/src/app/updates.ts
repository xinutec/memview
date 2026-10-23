import { Injectable, signal } from '@angular/core';

import { BUILD_INFO } from './build-info';

/**
 * Reload when the runner serves another build. No service worker, so the
 * fingerprint in `/api/state` is the only signal: the build time stamped into the
 * served bundle, compared with this page's own. Not with the first one heard: a
 * page the WebView restores from its cache on Back is an old build, and took the
 * new fingerprint for its own.
 *
 * The timing follows the life app's, so a half-typed instruction is never lost:
 * at startup or hidden, reload at once; visible mid-session, wait until the app
 * is next put away; restored by going back, treat it as a boot. No reload loop:
 * a build a reload did not reach is shown as held instead.
 */
@Injectable({ providedIn: 'root' })
export class Updates {
  /** Updates arriving this soon after boot reload immediately. */
  private static readonly STARTUP_MS = 10_000;
  /** A reload for the same build again within this is a loop. */
  private static readonly LOOP_MS = 30_000;
  /** Where the last reload's fingerprint and time are kept, across it. */
  private static readonly RELOADED_FOR = 'console.reloaded-for';

  private booted = Date.now();
  /** The build this page is. Assignable so a test can be any build. */
  own: string = BUILD_INFO.builtAt;
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
    if (!bundle || bundle === this.own) return;
    const starting = Date.now() - this.booted < Updates.STARTUP_MS;
    if ((starting || document.visibilityState === 'hidden') && this.firstFor(bundle)) {
      this.reload();
    } else this.pending.set(true);
  }

  /**
   * Whether no reload for `bundle` happened in the last [[Updates.LOOP_MS]]: one
   * that did and landed here again cannot reach that build. Going back to an old
   * page later is a new reason. Without storage, no: a loop could not be stopped.
   */
  private firstFor(bundle: string): boolean {
    try {
      const [last, at] = (sessionStorage.getItem(Updates.RELOADED_FOR) ?? '').split(' ');
      if (last === bundle && Date.now() - Number(at) < Updates.LOOP_MS) return false;
      sessionStorage.setItem(Updates.RELOADED_FOR, `${bundle} ${Date.now()}`);
      return true;
    } catch {
      return false;
    }
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
