import { Component, inject } from '@angular/core';
import { MAT_BOTTOM_SHEET_DATA } from '@angular/material/bottom-sheet';

import { lines } from './diff';
import type { Change } from './models';

/** What the sheet is opened with: the file an `Edit` changed, and the change. */
export interface Edited {
  readonly path: string;
  readonly change: Change;
}

/** What each kind of line is marked with, so it reads without colour too. */
const MARK = { same: ' ', gone: '−', added: '+' } as const;

/** One `Edit`, as a diff: the file on top, the lines it removed and added below. */
@Component({
  selector: 'app-diff-sheet',
  templateUrl: './diff-sheet.html',
  styleUrl: './diff-sheet.scss',
})
export class DiffSheet {
  protected readonly edited = inject<Edited>(MAT_BOTTOM_SHEET_DATA);
  protected readonly lines = lines(this.edited.change);
  protected readonly mark = MARK;
  protected readonly file = this.edited.path.split('/').at(-1) ?? this.edited.path;
  protected readonly folder = this.edited.path.slice(0, -this.file.length);
}
