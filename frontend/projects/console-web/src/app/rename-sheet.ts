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
   * A few words a model wrote for this conversation, when there are any — the
   * second line of the gist call, and exactly as much of a guess.
   */
  readonly suggestion?: string;
}

/**
 * The suggestion to show, given what a model wrote and what the box says now.
 * Withheld once it is what the box already says — a control that does nothing
 * reads as broken — which also covers the name the conversation already has.
 */
export function offered(suggestion: string | undefined, current: string): string | undefined {
  const name = suggestion?.trim();
  return name && name !== current.trim() ? name : undefined;
}

/**
 * Name a conversation. The console needs this because `/rename` is input: sent
 * to a working session it is parked and released as a prompt the model reads
 * as words ("nothing for me to do"). A sheet: one field, one button, dismisses
 * itself.
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

  /**
   * Prefilled with what it is called. Empty for a session never named: the list
   * shows `Code · 3f8a1c2b`, and prefilling an id to clear is worse than an empty box.
   */
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
      // Nothing is set from the response: the CLI writes the name to the transcript
      // and the runner reads it from there, so it arrives on the next poll.
      next: () => this.sheet.dismiss(),
      error: (err: unknown) => {
        this.saving.set(false);
        this.trouble.set(reason(err));
      },
    });
  }
}
