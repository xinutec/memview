import { Component, computed, inject, signal } from '@angular/core';
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
  /**
   * A few words a model wrote for this conversation, when there are any.
   *
   * The second line of the gist call — so it costs nothing to have here, and it
   * is exactly as much of a guess as the sentence on the card. Absent for a
   * conversation with no gist yet, and for one whose gist predates the second
   * line.
   */
  readonly suggestion?: string;
}

/**
 * The suggestion to show, given what a model wrote and what the box says now.
 *
 * ⚠ **Withheld once it is what the box already says.** Offering a suggestion
 * that has been taken invites a second tap that does nothing, and a control that
 * does nothing reads as a control that is broken. Compared trimmed, because the
 * box is what the person has been typing in and a trailing space is not a
 * different name.
 *
 * ⚠ **Also withheld when it matches the name the conversation already has** —
 * that falls out of the same comparison, since the box opens holding it. A
 * session whose name a model would have chosen anyway is the one case where a
 * suggestion is certain to be useless.
 */
export function offered(suggestion: string | undefined, current: string): string | undefined {
  const name = suggestion?.trim();
  return name && name !== current.trim() ? name : undefined;
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

  /** The name a model would give this conversation. See [[offered]]. */
  protected readonly suggestion = computed(() => offered(this.given.suggestion, this.title()));

  /** Take the suggestion, leaving it in the box to be edited or sent. */
  protected accept(name: string): void {
    this.title.set(name);
  }

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
