import { DOCUMENT, Injectable, OnDestroy, inject } from '@angular/core';

/**
 * How long to wait before each attempt, and — by its length — how many there are.
 *
 * Rising, because the two ways a stylesheet fails want different patience. A
 * dropped connection is over in a moment; a runner mid-rebuild is not. Bounded
 * rather than endless: a file that is genuinely gone will not arrive, and a page
 * quietly re-requesting it forever is worse than one that has stopped.
 */
export const BACKOFF_MS = [500, 2_000, 8_000];

/** Where the inline recorder in `index.html` leaves what broke before boot. */
declare global {
  interface Window {
    brokenAssets?: string[];
  }
}

/**
 * Ask again for a stylesheet whose request failed.
 *
 * **Why this exists.** The console reported `broke styles-*.css` from a phone on
 * a reload, alongside an `/api/state` that failed with status 0 — one dropped
 * connection taking both. The API call was retried by the polling that would
 * have made it anyway; the stylesheet was not asked for a second time by
 * anything, so the app ran correctly and completely unstyled until it was
 * reloaded by hand. Every icon on the page is a font ligature, so an unstyled
 * console is one showing the *words* `more_vert` and `send` where its buttons
 * were.
 *
 * ⚠ **The failure usually happens before this class exists.** Stylesheets are
 * requested during parsing and module scripts run after it, so by the time
 * Angular boots the error has already been and gone. The inline script in
 * `index.html` is what catches those; this drains what it recorded. Anything
 * failing later — a lazy chunk, a retry of our own — is caught here directly.
 */
@Injectable({ providedIn: 'root' })
export class Restyle implements OnDestroy {
  private readonly doc = inject(DOCUMENT);
  /** Attempts so far, by the stylesheet's own address without our query. */
  private readonly tried = new Map<string, number>();
  private started = false;
  /** Kept so the listener can be taken off again; see [`ngOnDestroy`]. */
  private listener?: (event: Event) => void;

  /**
   * Called once from the app shell; idempotent.
   *
   * The guard is not ceremony. A second listener does not retry twice as hard —
   * it retries twice *per failure*, doubling the requests aimed at whatever is
   * already struggling, and it exhausts the attempt bound in half the rounds.
   */
  init(): void {
    if (this.started) return;
    this.started = true;
    const view = this.doc.defaultView;
    for (const href of view?.brokenAssets ?? []) this.again(href);
    if (view) view.brokenAssets = [];

    this.listener = (event: Event) => {
      const target = event.target;
      if (target instanceof HTMLLinkElement && target.rel === 'stylesheet') {
        // A replacement that failed is a dead element — it styles nothing, and
        // left in place it would be counted again by every later attempt.
        if (target.dataset['again']) target.remove();
        this.again(target.getAttribute('href') ?? '');
      }
    };
    view?.addEventListener('error', this.listener, { capture: true });
  }

  /**
   * Take the listener off again.
   *
   * ⚠ **The window outlives this service.** In the app that is a distinction
   * without a difference — one page, one instance, both ending together — but a
   * listener holding a destroyed service is a leak wherever the injector is
   * rebuilt and the window is not, which is exactly what a test suite does.
   * Without this, each suite left another live listener on the same window and
   * one failed stylesheet was answered several times over.
   */
  ngOnDestroy(): void {
    const view = this.doc.defaultView;
    if (this.listener) view?.removeEventListener('error', this.listener, { capture: true });
    this.listener = undefined;
    this.started = false;
  }

  /**
   * Request one stylesheet again, later.
   *
   * Appended to the head rather than put back where the old one was: there is
   * one stylesheet in this app, and the last rule wins anyway. The failed link
   * is left alone — it is inert, and removing it would be a second thing that
   * could go wrong on a page already having a bad time.
   */
  private again(href: string): void {
    // Ours carry a query so a browser holding a failed response does not answer
    // from it. Counting has to ignore that, or each attempt would look like a
    // different file and the bound would never be reached.
    const asset = href.split('?')[0];
    if (!asset) return;
    const attempt = (this.tried.get(asset) ?? 0) + 1;
    if (attempt > BACKOFF_MS.length) return;
    this.tried.set(asset, attempt);

    const view = this.doc.defaultView;
    view?.setTimeout(() => {
      const fresh = this.doc.createElement('link');
      fresh.rel = 'stylesheet';
      fresh.href = `${asset}?again=${attempt}`;
      fresh.dataset['again'] = String(attempt);
      this.doc.head.appendChild(fresh);
    }, BACKOFF_MS[attempt - 1]);
  }
}
