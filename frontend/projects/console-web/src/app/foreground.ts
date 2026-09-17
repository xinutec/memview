import { DestroyRef, Injectable } from '@angular/core';

/**
 * When the app comes back to the front. Android freezes a cached app —
 * `setInterval` stops — and the page that comes back is whatever arrived before
 * the phone went in a pocket, so every poll pairs with this. `visibilitychange`
 * rather than `focus`: a WebView loses visibility when its activity stops, and
 * a keyboard taking focus is not that.
 */
@Injectable({ providedIn: 'root' })
export class Foreground {
  private waiting = new Set<() => void>();

  constructor() {
    document.addEventListener('visibilitychange', () => {
      if (document.visibilityState === 'visible') {
        for (const refresh of this.waiting) refresh();
      }
    });
  }

  /**
   * Run `refresh` each time the app returns to the front, until `until` is gone.
   * The lifetime is passed in: this is called from a constructor, where `inject`
   * works by luck of the call stack.
   */
  onReturn(refresh: () => void, until: DestroyRef): void {
    this.waiting.add(refresh);
    until.onDestroy(() => this.waiting.delete(refresh));
  }
}
