import { Component, inject, signal } from '@angular/core';
import { MAT_BOTTOM_SHEET_DATA, MatBottomSheetRef } from '@angular/material/bottom-sheet';
import { MatIconModule } from '@angular/material/icon';

import { type Mode } from './models';
import { type Known, offeredModes } from './modes';

/** Which session is being set, and what it is on now. */
export interface Choosing {
  readonly id: string;
  readonly mode?: Mode;
}

/**
 * What a session may do without asking. Its own sheet rather than six rows in
 * the ⋮ menu, which they were most of. Still the one place the escalation order
 * and the icons are shown together.
 */
@Component({
  selector: 'app-modes-sheet',
  templateUrl: './modes-sheet.html',
  styleUrl: './modes-sheet.scss',
  imports: [MatIconModule],
})
export class ModesSheet {
  private readonly given = inject<Choosing>(MAT_BOTTOM_SHEET_DATA);
  private sheet = inject(MatBottomSheetRef<ModesSheet, string>);

  protected readonly modes = offeredModes();
  /** What it is on, as far as this sheet knows. See [[App.setMode]] for who records it. */
  protected readonly chosen = signal(this.given.mode);

  protected pick(mode: Known): void {
    // Dismissed WITH the choice rather than calling the API here: the toolbar owns
    // the optimistic set and the rollback.
    this.sheet.dismiss(mode);
  }
}
