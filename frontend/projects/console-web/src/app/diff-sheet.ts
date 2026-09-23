import { Component, inject } from '@angular/core';
import { MAT_BOTTOM_SHEET_DATA } from '@angular/material/bottom-sheet';

import { DiffView } from './diff-view';
import type { Change } from './models';

/** One `Edit`, as a diff: the file on top, the lines it removed and added below. */
@Component({
  selector: 'app-diff-sheet',
  templateUrl: './diff-sheet.html',
  styleUrl: './diff-sheet.scss',
  imports: [DiffView],
})
export class DiffSheet {
  protected readonly change = inject<Change>(MAT_BOTTOM_SHEET_DATA);
  protected readonly file = this.change.path.split('/').at(-1) ?? this.change.path;
  protected readonly folder = this.change.path.slice(0, -this.file.length);
}
