import { Component, computed, inject, signal } from '@angular/core';
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
 * Start a session, from behind the one control that offers it. A sheet: the
 * list is what the console is opened for and starting is rare. It carries its own trouble, since a refusal
 * — a directory outside the allow list — is answered by editing the field.
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
   * Prefilled with the directory conversations are actually started in, not
   * `repos[0]`: alphabetically first is plausible-looking and wrong.
   */
  protected readonly dir = signal(this.given.common ?? this.repos[0] ?? '');

  /**
   * The repositories worth offering for what has been typed so far, matched on
   * the last path element: every repository lives under `~/Code`, which is what
   * the field opens on, so a whole-value match offers all of them at once.
   */
  protected readonly suggestions = computed(() => {
    const whole = this.dir().trim();
    // Nothing to suggest once the answer is typed.
    if (this.repos.some((repo) => repo === whole)) return [];
    const typed = whole.split('/').filter(Boolean).at(-1)?.toLowerCase() ?? '';
    return this.repos.filter((repo) => this.shortened(repo).toLowerCase().includes(typed));
  });

  /** What a repository is called, which is the only part worth reading in a
   *  list where every row shares the same parent. */
  protected shortened(repo: string): string {
    return repo.split('/').filter(Boolean).at(-1) ?? repo;
  }
  protected readonly starting = signal(false);
  protected readonly trouble = signal('');

  protected start(): void {
    const dir = this.dir().trim();
    if (!dir || this.starting()) return;
    this.starting.set(true);
    this.trouble.set('');
    // No opening instruction: the sheet navigates straight to the session.
    this.api.start(dir, '').subscribe({
      next: (session) => {
        this.starting.set(false);
        // Closed before navigating: the sheet is a sibling of the router outlet.
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
