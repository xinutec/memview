import { DestroyRef, Injectable } from '@angular/core';

/**
 * When the app comes back to the front.
 *
 * ⚠ **A poll is not a promise that the page is current.** Android freezes a
 * cached app: the process is suspended, `setInterval` stops firing, and the page
 * that comes back on screen is whatever arrived before the phone went in a
 * pocket — with nothing on it to say so. The console showed a conversation as
 * *in use* for minutes after it had ended, and the page was not wrong about what
 * it had been told; it had simply not been told since. From the reader's side
 * those are the same thing, which is what makes it worth handling rather than
 * explaining.
 *
 * So every poll pairs with this. Coming to the front is the one moment somebody
 * is certainly looking, and therefore the moment the answer has to be true.
 *
 * `visibilitychange` rather than `focus`: a WebView loses document visibility
 * when its activity stops, which is exactly the event that precedes the freeze,
 * and it does not also fire for a keyboard or a dialog taking focus.
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
   *
   * The lifetime is passed in rather than injected here: this is called from a
   * component's constructor, where `inject` still works by luck of the call
   * stack, and a helper that silently depends on that breaks when somebody moves
   * the call one line later.
   */
  onReturn(refresh: () => void, until: DestroyRef): void {
    this.waiting.add(refresh);
    until.onDestroy(() => this.waiting.delete(refresh));
  }
}
