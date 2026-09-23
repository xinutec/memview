import { Component, computed, input } from '@angular/core';

import { lines } from './diff';
import type { Change } from './models';

/** What each kind of line is marked with, so it reads without colour too. */
const MARK = { same: ' ', gone: '−', added: '+' } as const;

/** One change as diff lines: removed and added, unchanged lines muted. */
@Component({
  selector: 'app-diff',
  templateUrl: './diff-view.html',
  styleUrl: './diff-view.scss',
})
export class DiffView {
  readonly change = input.required<Change>();
  protected readonly lines = computed(() => lines(this.change()));
  protected readonly mark = MARK;
}
