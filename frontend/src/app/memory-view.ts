import { Component, inject, input, signal } from '@angular/core';
import { DatePipe } from '@angular/common';
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

  constructor() {
    toObservable(this.name)
      .pipe(
        switchMap((n) => {
          this.loading.set(true);
          return this.api.memory(n).pipe(catchError(() => of(null)));
        }),
        takeUntilDestroyed(),
      )
      .subscribe((page) => {
        this.page.set(page);
        this.missing.set(page === null);
        this.loading.set(false);
      });
  }
}
