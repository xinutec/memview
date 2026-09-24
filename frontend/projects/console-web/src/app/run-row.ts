import { Component, computed, input } from '@angular/core';
import { MatIconModule } from '@angular/material/icon';

import { Folding } from './folding';
import { Lasted } from './lasted';
import { type Block, ran } from './transcript';

/** A run of tool calls folded into one row, which opens it. */
@Component({
  selector: 'li[app-run]',
  templateUrl: './run-row.html',
  styleUrl: './run-row.scss',
  host: { class: 'entry tools' },
  imports: [Lasted, MatIconModule],
})
export class RunRow {
  readonly block = input.required<Block & { kind: 'tools' }>();
  readonly folding = input.required<Folding>();
  /** A clock the parent ticks while something is running. */
  readonly now = input(0);

  protected readonly counted = computed(() => ran(this.block().entries));

  /** How long the oldest call still running has been going. */
  protected readonly runningFor = computed(() => {
    const oldest = this.block()
      .entries.filter(
        (entry) => entry.ok === undefined && !entry.unrecorded && entry.at !== undefined,
      )
      .map((entry) => entry.at ?? 0)
      .sort((a, b) => a - b)[0];
    return oldest === undefined ? undefined : this.now() - oldest;
  });
}
