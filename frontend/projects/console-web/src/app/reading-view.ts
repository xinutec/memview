import { Component, OnDestroy, computed, inject, signal } from '@angular/core';
import { DecimalPipe } from '@angular/common';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { catchError, of } from 'rxjs';

import { ConsoleApi } from './console-api';
import { Here } from './here';
import { CorpusRead } from './models';

/** One bar: a shape of work and its share of the biggest. */
interface Shape {
  name: string;
  n: number;
  share: number;
  gap: boolean;
}

/** Shapes whose bar would round to nothing still get a hairline. */
const HAIRLINE = 0.006;

/**
 * What the reader makes of every shell command the fleet has run.
 *
 * ⚠ **Its own screen, because it is not what the console is for.** This began as
 * a strip above the session list, where it was the second thing on a page whose
 * job is the conversations — and it is not urgent, not about any session, and
 * read at most once a day. Everything else on that page answers *what should I
 * do now*; this answers *what has been done, ever*, which is a question you go
 * looking for rather than one that should be in the way.
 *
 * The name is the viewer's — one thing should not have two names across two
 * apps, and `/reader` there shows the same artefact from the same mine.
 */
@Component({
  selector: 'app-reading-view',
  imports: [DecimalPipe, MatIconModule, MatProgressBarModule],
  templateUrl: './reading-view.html',
  styleUrl: './reading-view.scss',
})
export class ReadingView implements OnDestroy {
  private api = inject(ConsoleApi);
  private here = inject(Here);

  readonly reading = signal<CorpusRead | undefined>(undefined);
  /** Set when the survey could not be fetched — including "never mined". */
  readonly failed = signal(false);

  constructor() {
    // The bar above is drawn from the route, before anything is fetched — see
    // [[Here.page]].
    this.here.page.set('Reader');
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

  ngOnDestroy(): void {
    this.here.page.set(undefined);
  }

  /**
   * Every shape, not the head of them.
   *
   * ⚠ **The strip this replaced showed five**, which was right for something
   * read on the way past and wrong here: a page somebody navigated to should not
   * make them wonder what the other sixteen were. The tail is where `not
   * understood` lives, which is the number the rest of the page is qualified by.
   */
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

  /** File uses no outcome can confirm — after `&&`, or inside an `if`. */
  readonly unconfirmable = computed(() => {
    const got = this.reading();
    if (!got) return 0;
    return got.always + got.on_success + got.sometimes - got.certain;
  });
}
