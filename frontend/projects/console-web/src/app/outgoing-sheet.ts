import { Component, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MAT_BOTTOM_SHEET_DATA, MatBottomSheetRef } from '@angular/material/bottom-sheet';
import { MatButtonModule } from '@angular/material/button';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';

import { Outbox, type Outgoing, standing } from './outbox';

/**
 * What can still be done with a message the Mac has not confirmed. One that never
 * left the phone is changed here; one that may already be there is put to the Mac,
 * which refuses an edit or a cancel once the original arrived — see [[Outbox]].
 */
@Component({
  selector: 'app-outgoing-sheet',
  templateUrl: './outgoing-sheet.html',
  styleUrl: './outgoing-sheet.scss',
  imports: [FormsModule, MatButtonModule, MatFormFieldModule, MatIconModule, MatInputModule],
})
export class OutgoingSheet {
  protected readonly message = inject<Outgoing>(MAT_BOTTOM_SHEET_DATA);
  private outbox = inject(Outbox);
  private sheet = inject(MatBottomSheetRef<OutgoingSheet>);

  protected readonly standing = standing(this.message, Date.now());
  protected readonly editing = signal(false);
  protected readonly text = signal(this.message.text);

  protected edit(): void {
    const text = this.text().trim();
    if (!text) return;
    void this.outbox.edit(this.message.id, text);
    this.sheet.dismiss();
  }

  protected cancel(): void {
    void this.outbox.cancel(this.message.id);
    this.sheet.dismiss();
  }

  protected resend(): void {
    void this.outbox.resend(this.message.id);
    this.sheet.dismiss();
  }

  protected discard(): void {
    void this.outbox.discard(this.message.id);
    this.sheet.dismiss();
  }
}
