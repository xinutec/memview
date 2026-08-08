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

/** What the list hands the sheet: where a session may be started, and where they
 *  actually are. */
export interface StartWhere {
  readonly repos: readonly string[];
  /** The commonest directory, or absent when nothing has ever been started. */
  readonly common?: string;
}

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
  /** The repositories the runner offers, and where conversations are actually
   *  started. See [[SessionsView.commonest]]. */
  private readonly given = inject<StartWhere>(MAT_BOTTOM_SHEET_DATA);
  /** For the field's own list of suggestions. */
  protected readonly repos = this.given.repos;

  private api = inject(ConsoleApi);
  private router = inject(Router);
  private sheet = inject(MatBottomSheetRef<StartSheet>);

  /**
   * Prefilled with the directory this machine's conversations are actually
   * started in — one less thing to type on a phone, and still editable, since
   * the field carries the whole list as its suggestions.
   *
   * ⚠ **It used to be `repos[0]`**, the first repository alphabetically. That is
   * a real directory and a plausible-looking default, which is what made it
   * quietly wrong: nothing had ever been started there, so the commonest action
   * was to notice and retype it.
   */
  protected readonly dir = signal(this.given.common ?? this.repos[0] ?? '');
  protected readonly starting = signal(false);
  protected readonly trouble = signal('');

  protected start(): void {
    const dir = this.dir().trim();
    if (!dir || this.starting()) return;
    this.starting.set(true);
    this.trouble.set('');
    // No opening instruction: the sheet navigates straight to the session, where
    // the composer is. See the note in the template.
    this.api.start(dir, '').subscribe({
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
