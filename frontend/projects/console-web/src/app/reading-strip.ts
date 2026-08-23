import { Component, computed, inject, signal } from '@angular/core';
import { DecimalPipe } from '@angular/common';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { catchError, of } from 'rxjs';

import { ConsoleApi } from './console-api';
import { CorpusRead } from './models';

/** One bar: a shape of work and its share of the biggest. */
interface Shape {
  name: string;
  n: number;
  share: number;
  gap: boolean;
}

/**
 * How many shapes the strip draws.
 *
 * ⚠ **Fewer than the viewer's, and the tail is not "the rest".** This is a strip
 * on a phone's front page, so it shows the head of the distribution and says the
 * total above it; the full twenty-two are a page away in the viewer. What it
 * must never do is imply these five are all of them.
 */
const SHOWN = 5;

/** Shapes whose bar would round to nothing still get a hairline. */
const HAIRLINE = 0.006;

/**
 * What the reader makes of the fleet's shell, in the space above the sessions.
 *
 * ⚠ **A fact about the whole corpus, so it sits with the other account-wide
 * facts** — beside usage, above the cards, not inside one. Nothing here is about
 * the conversation somebody came to the console for, which is also why it is
 * quieter than the cards and why it is only five rows.
 *
 * The numbers are the same artefact the viewer's `/reader` page draws and
 * `--bin shell-files` prints. One survey, three views: see
 * `reader/src/reading.rs`.
 */
@Component({
  selector: 'app-reading-strip',
  imports: [DecimalPipe],
  templateUrl: './reading-strip.html',
  styleUrl: './reading-strip.scss',
})
export class ReadingStrip {
  private api = inject(ConsoleApi);

  readonly reading = signal<CorpusRead | undefined>(undefined);
  /** Set when the survey could not be fetched — including "never mined". */
  readonly failed = signal(false);

  constructor() {
    // ⚠ **A failure says one muted line, not nothing.** The strip is not what
    // anybody opened the console for, so it stays quiet — but quiet is not the
    // same as absent, and an empty result that renders as a missing component is
    // indistinguishable from the component having been deleted.
    this.api
      .reading()
      .pipe(
        catchError(() => {
          this.failed.set(true);
          return of(undefined);
        }),
        takeUntilDestroyed(),
      )
      .subscribe((got) => {
        if (!got) return;
        this.failed.set(false);
        this.reading.set(got);
      });
  }

  readonly shapes = computed<Shape[]>(() => {
    const got = this.reading();
    if (!got) return [];
    const top = got.doing[0]?.n ?? 1;
    return got.doing.slice(0, SHOWN).map((row) => ({
      name: row.name,
      n: row.n,
      share: Math.max(row.n / top, HAIRLINE),
      gap: row.name === 'not understood',
    }));
  });
}
