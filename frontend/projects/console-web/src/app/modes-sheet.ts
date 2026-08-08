import { Component, inject, signal } from '@angular/core';
import { MAT_BOTTOM_SHEET_DATA, MatBottomSheetRef } from '@angular/material/bottom-sheet';
import { MatIconModule } from '@angular/material/icon';

import { offeredModes } from './modes';

/** Which session is being set, and what it is on now. */
export interface Choosing {
  readonly id: string;
  readonly mode?: string;
}

/**
 * What a session may do without asking.
 *
 * **Its own sheet rather than six rows in the ⋮ menu**, which is where these
 * lived. Six modes plus a heading was most of that menu — the two things anybody
 * opens it for, Details and Tasks, were above a wall of settings, and a menu that
 * long on a phone is scrolled rather than read.
 *
 * The escalation order and the icons are unchanged, and deliberately so: this is
 * still the only place both are shown together, so it is still where the icons
 * are learnt.
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
  /** What it is on, as far as this sheet knows. See [[App.setMode]] for who
   *  actually records it and why the answer is not read back from here. */
  protected readonly chosen = signal(this.given.mode);

  protected pick(mode: string): void {
    // Dismissed WITH the choice rather than calling the API here: the toolbar
    // already owns the optimistic set and the rollback on refusal, and two
    // places doing that would disagree about which one is showing.
    this.sheet.dismiss(mode);
  }
}
