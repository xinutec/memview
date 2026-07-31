import { Component, computed, inject, signal } from '@angular/core';
import { DatePipe } from '@angular/common';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { catchError, of } from 'rxjs';

import { MemviewApi } from './memview-api';
import { Agent, AgentsResult } from './models';

/** One project's share of an agent's work, as the list draws it. */
interface Place {
  project: string;
  reads: number;
  writes: number;
  /** Width of the bar, 0..1, against this agent's strongest project. */
  share: number;
}

/** An agent as the page shows it. */
interface AgentRow {
  name: string;
  /** True when the name was never set, so the id is standing in for it. */
  anonymous: boolean;
  reads: number;
  writes: number;
  first: string;
  last: string;
  places: Place[];
}

/** Projects listed per agent. Beyond this the tail is one-offs. */
const PLACES_SHOWN = 6;

/**
 * Which named session works on what.
 *
 * Several sessions run at once, each named for what it does. That name is a
 * claim; this page is the evidence — what each one actually opened and actually
 * changed, counted per project directory.
 *
 * **Writes decide where an agent lives, not reads.** Reading a repository is
 * consulting it; writing there is being responsible for it. On the live data
 * the difference is not cosmetic: the `health` agent reads the `pippijn`
 * monorepo more than anything else while doing its writing in `health`, so
 * ranking by reads would file it under the wrong project entirely.
 *
 * **And recent writes decide it, counted by days present rather than by files.**
 * A session is renamed as its job changes, so its name is a claim about now.
 * Ordering by lifetime file counts contradicted that: one session had spent a
 * single afternoon making seventy-five edits in a repository it never returned
 * to, and that afternoon outranked the fortnight of steady work it is named
 * for. The counts shown against each bar stay undecayed — they are the record
 * of what happened; only the order and the bar widths follow recency.
 *
 * Owner-only, and the server enforces it. These are counts rather than text,
 * but they describe the shape of the work — which projects exist and who is
 * doing what in them — and a share link is a deliberately public surface.
 */
@Component({
  selector: 'app-agents-view',
  templateUrl: './agents-view.html',
  styleUrl: './agents-view.scss',
  imports: [DatePipe, MatIconModule, MatProgressBarModule],
})
export class AgentsView {
  private api = inject(MemviewApi);

  readonly data = signal<AgentsResult | null>(null);
  readonly denied = signal(false);
  readonly loading = signal(true);

  readonly rows = computed<AgentRow[]>(() =>
    (this.data()?.agents ?? []).map((a) => this.row(a)),
  );

  readonly generated = computed(() => this.data()?.generated ?? '');

  constructor() {
    this.api
      .agents()
      .pipe(
        catchError(() => {
          // 403 is the expected answer for a share-token viewer, and a real
          // state to render rather than an error to swallow into an empty page
          // that reads as "nothing has been mined".
          this.denied.set(true);
          return of(null);
        }),
        takeUntilDestroyed(),
      )
      .subscribe((res) => {
        this.data.set(res);
        this.loading.set(false);
      });
  }

  private row(a: Agent): AgentRow {
    const projects = new Set([...Object.keys(a.reads), ...Object.keys(a.writes)]);
    const weight = (project: string) =>
      // Recent writing decides the ordering; recent reading only separates
      // places the agent has consulted without ever being responsible for.
      (a.recent_writes[project] ?? 0) + (a.recent_reads[project] ?? 0) / 1000;
    const places: Place[] = [...projects]
      .map((project) => ({
        project,
        reads: a.reads[project] ?? 0,
        writes: a.writes[project] ?? 0,
        share: weight(project),
      }))
      .sort((x, y) => y.share - x.share)
      .slice(0, PLACES_SHOWN);

    // Bars are scaled within the agent, not across all of them. Across, one
    // busy agent would flatten every other row to a sliver and the page would
    // only say "health is busiest" — which the totals already say.
    const strongest = Math.max(...places.map((p) => p.share), Number.MIN_VALUE);
    for (const p of places) p.share = p.share / strongest;

    const writes = Object.values(a.writes).reduce((n, v) => n + v, 0);
    const reads = Object.values(a.reads).reduce((n, v) => n + v, 0);
    return {
      name: a.name,
      // A session that was never named keeps its id — 36 characters of hex,
      // which is worth saying out loud rather than letting it read as a name.
      anonymous: /^[0-9a-f]{8}-[0-9a-f]{4}-/.test(a.name),
      reads,
      writes,
      first: a.first,
      last: a.last,
      places,
    };
  }
}
