import { Component, computed, inject, input } from '@angular/core';
import { MatButtonModule } from '@angular/material/button';
import { MatIconModule } from '@angular/material/icon';
import { Sheets } from '@xinutec/ui-scaffold';

import { lapseWarning } from './lapse';
import { SignInSheet } from './signin-sheet';

/**
 * The login every session shares, before it lapses and once it has: a warning
 * with the way to sign in again, so a lapse is not first noticed as a refused
 * session. See [[lapseWarning]] and [[SignInSheet]].
 */
@Component({
  selector: 'app-lapse-banner',
  templateUrl: './lapse-banner.html',
  styleUrl: './lapse-banner.scss',
  imports: [MatButtonModule, MatIconModule],
})
export class LapseBanner {
  private readonly sheets = inject(Sheets);
  /** When the login lapses, in epoch milliseconds, as the runner read it. */
  readonly lapses = input<number>();
  /** Recomputed with each state the runner sends, every few seconds. */
  readonly warning = computed(() => lapseWarning(this.lapses(), Date.now()));

  signIn(): void {
    this.sheets.open(SignInSheet, { panelClass: 'start-sheet' });
  }
}
