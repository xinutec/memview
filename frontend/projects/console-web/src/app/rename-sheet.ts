import { Component, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MAT_BOTTOM_SHEET_DATA, MatBottomSheetRef } from '@angular/material/bottom-sheet';
import { MatButtonModule } from '@angular/material/button';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';

import { ConsoleApi } from './console-api';
import { reason } from './errors';

/** Which conversation is being renamed, and what it is called now. */
export interface Renaming {
  readonly id: string;
  readonly title: string;
}

/**
 * Name a conversation, from the session it is about.
 *
 * **Why the console needs this at all, when the CLI has `/rename`.** A slash
 * command is *input*: written to stdin, parked by the CLI when it arrives
 * mid-turn, and released as a **prompt**. So a rename sent to a working session
 * reaches the model as words — measured 2026-08-08, the agent replied "Noted the
 * rename (CLI-side, nothing for me to do)" and no name was ever written. A
 * console whose sessions are usually working needs the other channel.
 *
 * A sheet, like starting a session: one field and one button, and it dismisses
 * itself so the answer arrives on the list behind it.
 */
@Component({
  selector: 'app-rename-sheet',
  templateUrl: './rename-sheet.html',
  styleUrl: './rename-sheet.scss',
  imports: [FormsModule, MatButtonModule, MatFormFieldModule, MatIconModule, MatInputModule],
})
export class RenameSheet {
  private readonly given = inject<Renaming>(MAT_BOTTOM_SHEET_DATA);
  private api = inject(ConsoleApi);
  private sheet = inject(MatBottomSheetRef<RenameSheet>);

  /** Prefilled with what it is called, because renaming is usually editing.
   *
   *  ⚠ Empty for a session that has never been named — the list shows
   *  `Code · 3f8a1c2b` for one of those, and prefilling an id somebody then has
   *  to clear is worse than an empty box. */
  protected readonly title = signal(this.given.title);
  protected readonly saving = signal(false);
  protected readonly trouble = signal('');

  protected save(): void {
    const title = this.title().trim();
    if (!title || this.saving()) return;
    this.saving.set(true);
    this.trouble.set('');
    this.api.rename(this.given.id, title).subscribe({
      // ⚠ **Nothing is set from the response.** The CLI writes the new name to
      // the transcript and the runner reads names from there, so it arrives on
      // the next poll rather than in this answer — and reporting the requested
      // name as the session's would be the console describing its own intent,
      // which is the defect five of this project's tasks were about.
      next: () => this.sheet.dismiss(),
      error: (err: unknown) => {
        this.saving.set(false);
        this.trouble.set(reason(err));
      },
    });
  }
}
