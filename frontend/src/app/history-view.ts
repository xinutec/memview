import { Component, computed, inject, signal } from '@angular/core';
import { DatePipe } from '@angular/common';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { ActivatedRoute, Router } from '@angular/router';
import { Subject, catchError, of, switchMap } from 'rxjs';

import { MemviewApi } from './memview-api';
import { HistoryHit, HistoryProject, HistorySummary } from './models';

/** A project as the list shows it: who worked on it and when. */
interface ProjectRow {
  name: string;
  turns: number;
  first: string;
  last: string;
  /** Days with at least one turn — not the span, which counts idle days. */
  activeDays: number;
  /** "home (58)", most turns first. */
  who: string;
  files: number;
}

/**
 * What every session actually worked on.
 *
 * The page exists because the obvious way to answer "who made heatcam" is
 * wrong. MEMORY.md names every project and is injected into every session's
 * context, so searching the transcripts for a project name matches nearly every
 * session ever run. Everything here is derived from `cwd` instead — where a
 * session actually was, which no injected context can fake.
 *
 * Owner-only, and the server enforces it. A share token gets 403 rather than a
 * thinner page: handing somebody a link to one memory must not hand them every
 * prompt ever typed.
 */
@Component({
  selector: 'app-history-view',
  templateUrl: './history-view.html',
  styleUrl: './history-view.scss',
  imports: [
    DatePipe,
    FormsModule,
    MatButtonModule,
    MatFormFieldModule,
    MatIconModule,
    MatInputModule,
    MatProgressBarModule,
  ],
})
export class HistoryView {
  private api = inject(MemviewApi);
  private router = inject(Router);
  private route = inject(ActivatedRoute);

  readonly summary = signal<HistorySummary | null>(null);
  readonly denied = signal(false);
  readonly query = signal('');
  readonly project = signal<string | null>(null);
  readonly hits = signal<HistoryHit[] | null>(null);
  readonly total = signal(0);
  readonly searching = signal(false);

  private search$ = new Subject<{ q: string; project: string | null }>();

  /** Session name by index, for resolving a project's hands. */
  private readonly sessionNames = computed(() =>
    (this.summary()?.sessions ?? []).map((s) => s.name),
  );

  readonly projects = computed<ProjectRow[]>(() => {
    const names = this.sessionNames();
    return (this.summary()?.projects ?? []).map((p: HistoryProject) => ({
      name: p.name,
      turns: p.turns,
      first: p.first,
      last: p.last,
      activeDays: p.days.length,
      // Three is enough to answer "who made this" — a fourth contributor with
      // two turns is noise in a list meant to be scanned.
      who: p.hands
        .slice(0, 3)
        .map((h) => `${names[h.session] ?? '?'} (${h.turns})`)
        .join(', '),
      files: p.files.length,
    }));
  });

  /** Whether the search returned more than the server was willing to send. */
  readonly truncated = computed(() => {
    const shown = this.hits()?.length ?? 0;
    return this.total() > shown;
  });

  constructor() {
    this.api
      .history()
      .pipe(
        catchError(() => {
          // 403 is the expected answer for a share-token viewer, and it is a
          // real state to render rather than an error to swallow into an empty
          // page that looks like "nothing has been mined".
          this.denied.set(true);
          return of(null);
        }),
        takeUntilDestroyed(),
      )
      .subscribe((s) => this.summary.set(s));

    this.search$
      .pipe(
        switchMap(({ q, project }) =>
          this.api
            .historySearch(q, project ?? undefined)
            .pipe(catchError(() => of({ hits: [] as HistoryHit[], total: 0 }))),
        ),
        takeUntilDestroyed(),
      )
      .subscribe((res) => {
        this.hits.set(res.hits);
        this.total.set(res.total);
        this.searching.set(false);
      });

    // The query and the project filter live in the URL, so a search is
    // linkable and the back button returns to it.
    this.route.queryParamMap.pipe(takeUntilDestroyed()).subscribe((pm) => {
      const q = pm.get('q') ?? '';
      const project = pm.get('project');
      this.query.set(q);
      this.project.set(project);
      if (q || project) {
        this.searching.set(true);
        this.search$.next({ q, project });
      } else {
        this.hits.set(null);
        this.total.set(0);
      }
    });
  }

  private go(q: string, project: string | null): void {
    void this.router.navigate([], {
      relativeTo: this.route,
      // Length-tested, not `??`: the empty string has to become null so the URL
      // of an unfiltered page carries no bare `?q=`, and `??` would keep it.
      queryParams: {
        q: q.length ? q : null,
        project: project?.length ? project : null,
      },
      queryParamsHandling: 'merge',
    });
  }

  submit(): void {
    this.go(this.query().trim(), this.project());
  }

  /** Clicking a project filters to it; clicking it again clears the filter. */
  pickProject(name: string): void {
    this.go(this.query(), this.project() === name ? null : name);
  }

  clear(): void {
    this.query.set('');
    this.go('', this.project());
  }
}
