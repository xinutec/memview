import { Location } from '@angular/common';
import { Injectable, inject } from '@angular/core';
import { MatBottomSheetRef } from '@angular/material/bottom-sheet';

/**
 * Let the back gesture close an overlay, and close *only* the overlay.
 *
 * ⚠ **Back was doing two things at once.** A sheet takes no part in history, so
 * a back press with one open went to the page underneath — and Material's
 * `closeOnNavigation` then dismissed the sheet on the way past. Measured in the
 * layout harness: open the details sheet on a session, press back, and you land
 * on the session LIST with the sheet gone. The gesture that means "put this
 * panel away" also threw away the conversation it was opened from. On the list
 * it is worse: the start sheet sits on the root, so back goes out of the app.
 *
 * The fix is to give the sheet a history entry of its own, so there is a step
 * for back to spend itself on. Nothing here closes anything: pressing back pops
 * the entry, and `closeOnNavigation` does the dismissing exactly as before.
 *
 * ⚠ **The entry has to be taken away again when the sheet closes some other
 * way**, which is the ordinary way — a tap on the backdrop. Otherwise the step
 * outlives the panel it stood for and the next back press is spent on nothing,
 * which reads as a phone that ignored the gesture.
 */
@Injectable({ providedIn: 'root' })
export class Dismiss {
  private location = inject(Location);

  /** Wire `ref` into history, until it is dismissed. */
  onBack(ref: MatBottomSheetRef<unknown>): void {
    // The same URL, so nothing routes: this is a step in history, not a place.
    // `path(true)` keeps the query and hash, which a session's URL may carry.
    this.location.go(this.location.path(true), '', { overlay: true });
    // Completes on its own after one emission, so there is nothing to unwind.
    ref.afterDismissed().subscribe(() => {
      // Only if the step is still there. When back is what closed the sheet it
      // has already been popped, and a second `back()` here would leave the page
      // as well — the exact fault this exists to fix, arriving by the other door.
      if (this.stepped()) this.location.back();
    });
  }

  /** Whether the entry on top of the stack is one of ours. */
  private stepped(): boolean {
    const state: unknown = this.location.getState();
    return typeof state === 'object' && state !== null && 'overlay' in state;
  }
}
