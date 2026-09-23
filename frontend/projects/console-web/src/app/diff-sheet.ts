import { Component, inject } from '@angular/core';
import { MAT_BOTTOM_SHEET_DATA } from '@angular/material/bottom-sheet';

import { lines } from './diff';
import type { Change } from './models';

/** What each kind of line is marked with, so it reads without colour too. */
const MARK = { same: ' ', gone: '−', added: '+' } as const;

/** One `Edit`, as a diff: the file on top, the lines it removed and added below. */
@Component({
  selector: 'app-diff-sheet',
  templateUrl: './diff-sheet.html',
  styleUrl: './diff-sheet.scss',
})
export class DiffSheet {
  protected readonly change = inject<Change>(MAT_BOTTOM_SHEET_DATA);
  protected readonly lines = lines(this.change);
  protected readonly mark = MARK;
  protected readonly file = this.change.path.split('/').at(-1) ?? this.change.path;
  protected readonly folder = this.change.path.slice(0, -this.file.length);
}
