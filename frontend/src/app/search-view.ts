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
import { SearchHit, SearchResult, WorkMatch } from './models';

/**
 * What a failed search yields. Spelled out whole rather than as a partial
 * literal so the type stays exact — a fallback missing a field makes every read
 * of it `any`, which is how a rename slips past the gate.
 */
const EMPTY_RESULT: SearchResult = { hits: [], relaxed: false };

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
  /**
   * The search did not run. ⚠ **A distinct state from "no hits", because the
   * empty list is a CLAIM** — the template answers it with "No matches.", which
   * says these memories are not there. Swallowing a 500 into that claim is how a
   * reader concludes a memory does not exist and writes a second one.
   */
  readonly failed = signal(false);
  /** The query matched nothing whole, so it was widened. Said out loud. */
  readonly relaxed = signal(false);
  readonly searching = signal(false);
  /**
   * Who has been changing files the query names. Empty for a share-link
   * recipient, whose 403 is the intended answer rather than a failure — the
   * panel simply is not theirs to see.
   */
  readonly workers = signal<WorkMatch[] | null>(null);
  /** Which rows have their file list open. Collapsed by default: the evidence
   *  is for checking an answer, not for reading past it. */
  readonly opened = signal<ReadonlySet<string>>(new Set());
  private search$ = new Subject<string>();
  private work$ = new Subject<string>();

  constructor() {
    this.search$
      .pipe(
        switchMap((q) =>
          this.api.search(q).pipe(
            catchError(() => {
              this.failed.set(true);
              return of(EMPTY_RESULT);
            }),
          ),
        ),
        takeUntilDestroyed(),
      )
      .subscribe((res) => {
        this.results.set(res.hits);
        this.relaxed.set(res.relaxed);
        this.searching.set(false);
      });

    // A separate stream, so the memories arrive when they arrive: the roster is
    // a much bigger artefact and waiting for it would slow the search down for
    // the sake of a panel beside it.
    //
    // ⚠ **This swallow makes no claim, which is why it stays a swallow.** The
    // panel renders only when the list is non-empty, so a failure here shows
    // nothing at all rather than asserting that nobody works on this — unlike
    // the hit list above, whose empty state is a sentence. A 403 is the
    // intended answer for a share-link recipient and reaches the same place.
    this.work$
      .pipe(
        switchMap((q) => this.api.work(q).pipe(catchError(() => of([])))),
        takeUntilDestroyed(),
      )
      .subscribe((found) => this.workers.set(found));

    // ?q is the source of truth: landing with a query (or navigating back to
    // one) runs it.
    this.route.queryParamMap.pipe(takeUntilDestroyed()).subscribe((pm) => {
      const q = pm.get('q') ?? '';
      this.query.set(q);
      this.opened.set(new Set());
      if (q) {
        this.searching.set(true);
        this.failed.set(false);
        this.search$.next(q);
        this.work$.next(q);
      } else {
        this.results.set(null);
        this.workers.set(null);
      }
    });
  }

  toggle(name: string): void {
    const next = new Set(this.opened());
    if (!next.delete(name)) next.add(name);
    this.opened.set(next);
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

  /**
   * Run the current query again. Not `submit()`: the URL already holds this
   * query, so navigating to it emits nothing and the failed search would sit
   * there looking like a verdict.
   */
  retry(): void {
    const q = this.query().trim();
    if (!q) return;
    this.searching.set(true);
    this.failed.set(false);
    this.search$.next(q);
    this.work$.next(q);
  }
}
