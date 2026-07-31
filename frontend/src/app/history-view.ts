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
import {
  HistoryHit,
  HistoryProject,
  HistorySearchResult,
  HistorySummary,
  HistoryTally,
} from './models';

/**
 * What a failed search yields. Spelled out as a whole result rather than a
 * partial literal so the type stays exact — a fallback missing a field makes
 * every read of it `any`, which is how a rename would slip through the gate.
 */
const EMPTY_RESULT: HistorySearchResult = {
  hits: [],
  total: 0,
  by_session: [],
  by_project: [],
};

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
  /** Which sessions and projects the whole match set lives in. */
  readonly bySession = signal<HistoryTally[]>([]);
  readonly byProject = signal<HistoryTally[]>([]);
  readonly session = signal<string | null>(null);
  readonly searching = signal(false);

  private search$ = new Subject<{ q: string; project: string | null; session: string | null }>();

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
        switchMap(({ q, project, session }) =>
          this.api
            .historySearch(q, project ?? undefined, session ?? undefined)
            .pipe(catchError(() => of(EMPTY_RESULT))),
        ),
        takeUntilDestroyed(),
      )
      .subscribe((res) => {
        this.hits.set(res.hits);
        this.total.set(res.total);
        // dev-lint: allow-component-list search results belong to the current
        // query — surviving navigation would show one search's tallies beside
        // another's hits, which is worse than re-fetching.
        this.bySession.set(res.by_session);
        this.byProject.set(res.by_project);
        this.searching.set(false);
      });

    // The query and the project filter live in the URL, so a search is
    // linkable and the back button returns to it.
    this.route.queryParamMap.pipe(takeUntilDestroyed()).subscribe((pm) => {
      const q = pm.get('q') ?? '';
      const project = pm.get('project');
      const session = pm.get('session');
      this.query.set(q);
      this.project.set(project);
      this.session.set(session);
      if (q || project || session) {
        this.searching.set(true);
        this.search$.next({ q, project, session });
      } else {
        this.hits.set(null);
        this.total.set(0);
        this.bySession.set([]);
        this.byProject.set([]);
      }
    });
  }

  private go(q: string, project: string | null, session: string | null): void {
    void this.router.navigate([], {
      relativeTo: this.route,
      // Length-tested, not `??`: the empty string has to become null so the URL
      // of an unfiltered page carries no bare `?q=`, and `??` would keep it.
      queryParams: {
        q: q.length ? q : null,
        project: project?.length ? project : null,
        session: session?.length ? session : null,
      },
      queryParamsHandling: 'merge',
    });
  }

  submit(): void {
    this.go(this.query().trim(), this.project(), this.session());
  }

  /** Clicking a project filters to it; clicking it again clears the filter. */
  pickProject(name: string): void {
    this.go(this.query(), this.project() === name ? null : name, this.session());
  }

  /** Same for a session — this is how a tally row becomes a drill-down. */
  pickSession(name: string): void {
    this.go(this.query(), this.project(), this.session() === name ? null : name);
  }

  clear(): void {
    this.query.set('');
    this.go('', this.project(), this.session());
  }
}
