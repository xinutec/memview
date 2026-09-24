import { Location } from '@angular/common';
import { Injectable, inject } from '@angular/core';
import { MatBottomSheetRef } from '@angular/material/bottom-sheet';

/**
 * Let the back gesture close an overlay, and close only the overlay.
 *
 * A sheet takes no part in history, so back goes to the page underneath and
 * Material's `closeOnNavigation` dismisses the sheet on the way past — on the
 * list, out of the app. So the sheet gets a history entry of its own for back
 * to spend itself on; nothing here closes anything. The entry has to be
 * taken away again when the sheet closes some other way, or the next back
 * press is spent on nothing.
 */
@Injectable({ providedIn: 'root' })
export class Dismiss {
  private location = inject(Location);

  /** Wire `ref` into history, until it is dismissed. */
  onBack(ref: MatBottomSheetRef<unknown>): void {
    // The same URL, so nothing routes: a step in history, not a place. `path(true)`
    // keeps the query and hash.
    this.location.go(this.location.path(true), '', { overlay: true });
    // Completes on its own after one emission, so there is nothing to unwind.
    ref.afterDismissed().subscribe(() => {
      // Only if the step is still there: when back is what closed the sheet it has
      // already been popped, and a second `back()` would leave the page.
      if (this.stepped()) this.location.back();
    });
  }

  /** Whether the entry on top of the stack is one of ours. */
  private stepped(): boolean {
    const state: unknown = this.location.getState();
    return typeof state === 'object' && state !== null && 'overlay' in state;
  }
}
