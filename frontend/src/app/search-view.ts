import { Component, inject, signal } from '@angular/core';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { ActivatedRoute, Router, RouterLink } from '@angular/router';
import { Subject, catchError, of, switchMap } from 'rxjs';

import { MemviewApi } from './memview-api';
import { SearchHit } from './models';

/**
 * Full-text search. The query lives in ?q= so results are linkable and the
 * back button returns to them; requests run through switchMap so a slow
 * response can't land after a newer one.
 */
@Component({
  selector: 'app-search-view',
  templateUrl: './search-view.html',
  styleUrl: './search-view.scss',
  imports: [
    FormsModule,
    RouterLink,
    MatButtonModule,
    MatFormFieldModule,
    MatIconModule,
    MatInputModule,
    MatProgressBarModule,
  ],
})
export class SearchView {
  private api = inject(MemviewApi);
  private router = inject(Router);
  private route = inject(ActivatedRoute);

  readonly query = signal('');
  readonly results = signal<SearchHit[] | null>(null);
  readonly searching = signal(false);
  private search$ = new Subject<string>();

  constructor() {
    this.search$
      .pipe(
        switchMap((q) => this.api.search(q).pipe(catchError(() => of({ hits: [] as SearchHit[] })))),
        takeUntilDestroyed(),
      )
      .subscribe((res) => {
        this.results.set(res.hits);
        this.searching.set(false);
      });

    // ?q is the source of truth: landing with a query (or navigating back to
    // one) runs it.
    this.route.queryParamMap.pipe(takeUntilDestroyed()).subscribe((pm) => {
      const q = pm.get('q') ?? '';
      this.query.set(q);
      if (q) {
        this.searching.set(true);
        this.search$.next(q);
      } else {
        this.results.set(null);
      }
    });
  }

  submit(): void {
    const q = this.query().trim();
    void this.router.navigate([], {
      relativeTo: this.route,
      queryParams: { q: q || null },
      queryParamsHandling: 'merge',
    });
  }

  clear(): void {
    this.query.set('');
    this.submit();
  }
}
