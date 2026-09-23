import { DOCUMENT, Injectable, OnDestroy, inject } from '@angular/core';

/**
 * How long to wait before each attempt, and by its length how many there are.
 * Rising, since a dropped connection is over in a moment and a runner
 * mid-rebuild is not; bounded, since a file that is gone will not arrive.
 */
export const BACKOFF_MS = [500, 2_000, 8_000] as const;

/** Where the inline recorder in `index.html` leaves what broke before boot. */
declare global {
  interface Window {
    brokenAssets?: string[];
  }
}

/**
 * Ask again for a stylesheet whose request failed. Without it a dropped
 * connection on reload leaves the app unstyled — every icon is a font ligature,
 * so the buttons read `more_vert` and `send`. The failure usually happens before this class
 * exists, so the inline script in `index.html` records it and this drains what
 * it recorded.
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
   * Called once from the app shell; idempotent. A second listener retries twice
   * PER FAILURE and exhausts the bound in half the rounds.
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
        // A replacement that failed is a dead element, and left in place would be
        // counted again.
        if (target.dataset['again']) target.remove();
        this.again(target.getAttribute('href') ?? '');
      }
    };
    view?.addEventListener('error', this.listener, { capture: true });
  }

  /**
   * Take the listener off again. The window outlives this service wherever the
   * injector is rebuilt and the window is not — a test suite.
   */
  ngOnDestroy(): void {
    const view = this.doc.defaultView;
    if (this.listener) view?.removeEventListener('error', this.listener, { capture: true });
    this.listener = undefined;
    this.started = false;
  }

  /**
   * Request one stylesheet again, later. Appended to the head: there is one
   * stylesheet and the last rule wins. The failed link is left alone; it is inert.
   */
  private again(href: string): void {
    // Ours carry a query so a browser holding a failed response does not answer
    // from it; counting ignores it, or the bound would never be reached.
    const asset = href.split('?')[0];
    if (!asset) return;
    const attempt = (this.tried.get(asset) ?? 0) + 1;
    if (attempt > BACKOFF_MS.length) return;
    this.tried.set(asset, attempt);

    const view = this.doc.defaultView;
    view?.setTimeout(
      () => {
        const fresh = this.doc.createElement('link');
        fresh.rel = 'stylesheet';
        fresh.href = `${asset}?again=${attempt}`;
        fresh.dataset['again'] = String(attempt);
        this.doc.head.appendChild(fresh);
      },
      BACKOFF_MS[attempt - 1],
    );
  }
}
