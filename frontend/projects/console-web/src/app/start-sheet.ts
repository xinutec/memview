import { Component, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MAT_BOTTOM_SHEET_DATA, MatBottomSheetRef } from '@angular/material/bottom-sheet';
import { MatButtonModule } from '@angular/material/button';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';
import { Router } from '@angular/router';

import { ConsoleApi } from './console-api';
import { reason } from './errors';

/**
 * Start a session, from behind the one control that offers it.
 *
 * **A sheet rather than a card on the page, because of how often this is
 * wanted.** The list is what the console is opened for — a dozen conversations,
 * scanned for the one that is waiting — and starting a new one is rare beside
 * that. The form was the first thing on the page and the first thing scrolled
 * past, taking a screenful of a phone with it every time.
 *
 * It carries its own trouble rather than reporting up: this is the one place its
 * failures can be read without the sheet having to close first, and a refusal
 * here — a directory outside the allow list — is answered by editing the field
 * that caused it.
 */
@Component({
  selector: 'app-start-sheet',
  templateUrl: './start-sheet.html',
  styleUrl: './start-sheet.scss',
  imports: [FormsModule, MatButtonModule, MatFormFieldModule, MatIconModule, MatInputModule],
})
export class StartSheet {
  /** The repositories the runner offers, for the field's own list. */
  protected readonly repos = inject<readonly string[]>(MAT_BOTTOM_SHEET_DATA);

  private api = inject(ConsoleApi);
  private router = inject(Router);
  private sheet = inject(MatBottomSheetRef<StartSheet>);

  /** Prefilled with the first repository the runner offers, which is where the
   *  list used to start it from. One less thing to type on a phone, and still
   *  editable — the field carries the whole list as its own suggestions. */
  protected readonly dir = signal(this.repos[0] ?? '');
  protected readonly prompt = signal('');
  protected readonly starting = signal(false);
  protected readonly trouble = signal('');

  protected start(): void {
    const dir = this.dir().trim();
    if (!dir || this.starting()) return;
    this.starting.set(true);
    this.trouble.set('');
    this.api.start(dir, this.prompt().trim()).subscribe({
      next: (session) => {
        this.starting.set(false);
        // Closed before navigating: the sheet is a sibling of the router outlet
        // and would otherwise sit over the session it just started.
        this.sheet.dismiss();
        void this.router.navigate(['/s', session.id]);
      },
      error: (err: unknown) => {
        this.starting.set(false);
        this.trouble.set(reason(err));
      },
    });
  }
}
