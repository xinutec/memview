import { Component, computed, inject, signal } from '@angular/core';
import { DatePipe, DecimalPipe } from '@angular/common';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { catchError, of } from 'rxjs';

import { MemviewApi } from './memview-api';
import { Both, CorpusRead, Ranked } from './models';

/** One bar in the "what the shell was doing" chart. */
interface Shape {
  name: string;
  n: number;
  /** Width 0..1, against the biggest shape — not against the total. */
  share: number;
  /**
   * Whether this shape is an admission rather than an activity.
   *
   * ⚠ **`not understood` is drawn in the same chart as the rest, not beneath
   * it.** It is the size of the hole in this very chart, and a legend that
   * separated it would let a reader take the bars above as the whole picture.
   */
  gap: boolean;
}

/** Shapes whose bar would round to nothing are still listed, never dropped. */
const HAIRLINE = 0.004;

/**
 * What the reader makes of every shell command the fleet has run.
 *
 * The numbers `--bin shell-files` prints, drawn. Same artefact, same arithmetic:
 * the survey lives in `reader/src/reading.rs` and both the report and this page
 * are views of it, so the only way they can disagree is by being from different
 * nights.
 *
 * ⚠ **Coverage and its ceiling are on the same screen, deliberately.** 99.2% of
 * commands understood is the headline and 4.6% of file uses having no nameable
 * subject is the limit, and a page that showed the first without the second
 * would be advertising rather than reporting. They measure different
 * denominators — commands against uses — which is exactly why neither can stand
 * in for the other.
 */
@Component({
  selector: 'app-reader-view',
  imports: [DatePipe, DecimalPipe, MatIconModule, MatProgressBarModule],
  templateUrl: './reader-view.html',
  styleUrl: './reader-view.scss',
})
export class ReaderView {
  private api = inject(MemviewApi);

  readonly reading = signal<CorpusRead | undefined>(undefined);
  /** Set when the artefact has not been mined, or could not be fetched. */
  readonly missing = signal(false);

  constructor() {
    this.api
      .reading()
      .pipe(
        catchError(() => {
          this.missing.set(true);
          return of(undefined);
        }),
        takeUntilDestroyed(),
      )
      .subscribe((got) => {
        if (!got) return;
        // Withdrawn on success, not only raised on failure: a message that
        // outlives the condition it describes is worse than none, because it is
        // read as current.
        this.missing.set(false);
        this.reading.set(got);
      });
  }

  /** The chart, biggest first, with each bar scaled to the biggest. */
  readonly shapes = computed<Shape[]>(() => {
    const got = this.reading();
    if (!got) return [];
    const top = got.doing[0]?.n ?? 1;
    return got.doing.map((row) => ({
      name: row.name,
      n: row.n,
      share: Math.max(row.n / top, HAIRLINE),
      gap: row.name === 'not understood',
    }));
  });

  /**
   * File uses an outcome cannot confirm.
   *
   * Derived here rather than served, because it is a subtraction the server
   * would have to name and the name is the interesting part: these are uses
   * after `&&` or inside an `if`, where one exit status cannot say which way it
   * went.
   */
  readonly unconfirmable = computed(() => {
    const got = this.reading();
    if (!got) return 0;
    return got.always + got.on_success + got.sometimes - got.certain;
  });

  /** Milliseconds, for the date pipe. */
  readonly corpusAt = computed(() => {
    const at = this.reading()?.corpus_at;
    return at ? at * 1000 : undefined;
  });

  totalOf(rows: Both[]): number {
    return rows.reduce((sum, row) => sum + row.reads + row.writes, 0);
  }

  /** Widest count in a ranked list, so its bars share one scale. */
  topOf(rows: Ranked[]): number {
    return rows[0]?.n ?? 1;
  }
}
