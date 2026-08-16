import { Component, inject, input, signal } from '@angular/core';
import { DatePipe } from '@angular/common';
import { HttpErrorResponse } from '@angular/common/http';
import { takeUntilDestroyed, toObservable } from '@angular/core/rxjs-interop';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { RouterLink } from '@angular/router';
import { catchError, of, switchMap } from 'rxjs';

import { ContentNav } from './content-nav';
import { MemviewApi } from './memview-api';
import { MemoryPage } from './models';

/** One memory: frontmatter header, rendered body, out/backlinks. */
@Component({
  selector: 'app-memory-view',
  templateUrl: './memory-view.html',
  styleUrl: './memory-view.scss',
  imports: [DatePipe, RouterLink, ContentNav, MatProgressBarModule],
})
export class MemoryView {
  private api = inject(MemviewApi);

  /** Route param (:name), bound via withComponentInputBinding. */
  readonly name = input.required<string>();

  readonly page = signal<MemoryPage | null>(null);
  readonly missing = signal(false);
  readonly loading = signal(true);
  /**
   * The page did not load. ⚠ **Only a 404 means the memory is not there**, and
   * the difference is load-bearing here: `missing` renders "hasn't been written
   * yet — a dangling link marks something worth writing", which on a 500 is an
   * invitation to write a memory that already exists.
   */
  readonly failed = signal(false);

  constructor() {
    toObservable(this.name)
      .pipe(
        switchMap((n) => {
          this.loading.set(true);
          this.failed.set(false);
          return this.api.memory(n).pipe(
            catchError((err: unknown) => {
              this.failed.set(!(err instanceof HttpErrorResponse) || err.status !== 404);
              return of(null);
            }),
          );
        }),
        takeUntilDestroyed(),
      )
      .subscribe((page) => {
        this.page.set(page);
        this.missing.set(page === null && !this.failed());
        this.loading.set(false);
      });
  }
}
