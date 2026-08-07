import { Component, computed, inject, signal } from '@angular/core';
import { DatePipe } from '@angular/common';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { RouterLink } from '@angular/router';
import { catchError, of } from 'rxjs';

import { MemviewApi } from './memview-api';
import { Agent, AgentsResult, MemoryUse } from './models';

/** One project's share of an agent's work, as the list draws it. */
interface Place {
  project: string;
  /** Every use, tool call and Bash call together. */
  reads: number;
  writes: number;
  /** How much of the above came from a Bash call rather than `Write`/`Edit`. */
  shellReads: number;
  shellWrites: number;
  /**
   * Uses a command may never have made, and which are therefore in none of the
   * counts above. See [`AgentRow.maybeWrites`].
   */
  maybeReads: number;
  maybeWrites: number;
  /** Lines committed here — size, where the counts beside it are frequency. */
  added: number;
  deleted: number;
  /** Width of the bar, 0..1, against this agent's strongest project. */
  share: number;
}

/** Another machine this agent works on, from the `ssh` payloads it sent. */
interface Machine {
  host: string;
  reads: number;
  writes: number;
}

/** One memory an agent works with, as the list draws it. */
interface Known {
  name: string;
  reads: number;
  edits: number;
}

/** An agent as the page shows it. */
interface AgentRow {
  name: string;
  /** True when the name was never set, so the id is standing in for it. */
  anonymous: boolean;
  reads: number;
  writes: number;
  /** Subagent and workflow transcripts this session dispatched, if any. */
  delegated: number;
  first: string;
  last: string;
  /** The Bash share of the totals above, kept beside them and never inside. */
  shellReads: number;
  shellWrites: number;
  /**
   * Uses a command **may** have made, and which are in none of the counts above.
   *
   * ⚠ **Never added to them, and that is the whole point of the number.** A
   * command after `&&` runs only if what preceded it worked, and one exit status
   * for a script often cannot say whether it did — so these are files something
   * may never have touched. Summing them into `reads`/`writes` would spend the
   * distinction the miner went to the trouble of keeping.
   *
   * Only ever non-zero through the shell: measured over the artefact, `paths`,
   * `remote_paths` and `memories` carry 0, because a tool call is atomic and its
   * result says which way it went. That is why they are drawn inside the "through
   * the shell" phrase and nowhere else.
   */
  maybeReads: number;
  maybeWrites: number;
  added: number;
  deleted: number;
  commits: number;
  places: Place[];
  machines: Machine[];
  knows: Known[];
}

/** Uses folded onto one project: certain, and the ones only a shell can leave. */
interface Counted {
  reads: number;
  writes: number;
  maybeReads: number;
  maybeWrites: number;
}

const NOTHING = (): Counted => ({ reads: 0, writes: 0, maybeReads: 0, maybeWrites: 0 });

/** Machines listed per agent. */
const MACHINES_SHOWN = 4;

/**
 * The project a mined path belongs to — its first segment.
 *
 * A glob is recorded as it was written (`*​/*​/android`), which is honest for a
 * file list and useless as a project name, so those are dropped rather than
 * shown as a project called `*`.
 */
function projectOf(path: string): string | null {
  const head = path.split('/')[0];
  return head && !/[*?\[]/.test(head) ? head : null;
}

/** Projects listed per agent. Beyond this the tail is one-offs. */
const PLACES_SHOWN = 6;

/** Memories listed per agent. */
const KNOWS_SHOWN = 5;

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
 * **Work a session delegated counts as its own.** A subagent has no name and no
 * continuity; it ran because a named session asked it to. About a tenth of all
 * Read/Write/Edit calls in the corpus happen in delegated transcripts, and the
 * share runs from none at all to a seventh depending on the session — so leaving
 * them out would not just undercount but undercount unevenly, making agents
 * incomparable.
 *
 * **The memories under each row answer the other half of the question.** Where
 * an agent writes says which repository it owns; which memories it opens and
 * maintains says what it knows, which is the better evidence when a task is
 * unfamiliar ground for everyone.
 *
 * Owner-only, and the server enforces it. These are counts rather than text,
 * but they describe the shape of the work — which projects exist and who is
 * doing what in them — and a share link is a deliberately public surface.
 */
@Component({
  selector: 'app-agents-view',
  templateUrl: './agents-view.html',
  styleUrl: './agents-view.scss',
  imports: [DatePipe, MatIconModule, MatProgressBarModule, RouterLink],
})
export class AgentsView {
  private api = inject(MemviewApi);

  readonly data = signal<AgentsResult | null>(null);
  /** dev-lint: allow-sticky-error — not a failure to withdraw. A 403 here is the
   *  settled answer for a share-token viewer: the request is made once, nothing
   *  retries it, and the page renders this as a real state rather than an error.
   *  Clearing it would blank a correct page. */
  readonly denied = signal(false);
  readonly loading = signal(true);

  readonly rows = computed<AgentRow[]>(() => (this.data()?.agents ?? []).map((a) => this.row(a)));

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
    // Four dimensions of evidence, mined apart and unioned here — the same rule
    // /api/work follows. Kept apart in the artefact so the figures that were
    // always there go on meaning what they meant; unioned at the moment
    // somebody asks where an agent works, because that question wants all of it.
    const shell = this.byProject(a.shell_paths);
    const lines = this.linesByProject(a);
    const projects = new Set([
      ...Object.keys(a.reads),
      ...Object.keys(a.writes),
      ...shell.keys(),
      ...lines.keys(),
    ]);
    const weight = (project: string) =>
      // Recent writing decides the ordering; recent reading only separates
      // places the agent has consulted without ever being responsible for.
      (a.recent_writes[project] ?? 0) + (a.recent_reads[project] ?? 0) / 1000;
    const places: Place[] = [...projects]
      .map((project) => {
        const s = shell.get(project) ?? NOTHING();
        const l = lines.get(project) ?? { added: 0, deleted: 0 };
        return {
          project,
          reads: (a.reads[project] ?? 0) + s.reads,
          writes: (a.writes[project] ?? 0) + s.writes,
          shellReads: s.reads,
          shellWrites: s.writes,
          maybeReads: s.maybeReads,
          maybeWrites: s.maybeWrites,
          added: l.added,
          deleted: l.deleted,
          share: weight(project),
        };
      })
      .sort((x, y) => y.share - x.share || y.writes - x.writes)
      .slice(0, PLACES_SHOWN);

    // Bars are scaled within the agent, not across all of them. Across, one
    // busy agent would flatten every other row to a sliver and the page would
    // only say "health is busiest" — which the totals already say.
    const strongest = Math.max(...places.map((p) => p.share), Number.MIN_VALUE);
    for (const p of places) p.share = p.share / strongest;

    // Ranked by deliberate touches, edits breaking the tie: maintaining a
    // memory is a stronger claim to the ground it covers than consulting it.
    const knows: Known[] = Object.entries(a.memories ?? {})
      .map(([name, u]) => ({ name, ...u }))
      .sort(
        (x, y) =>
          y.reads + y.edits - (x.reads + x.edits) ||
          y.edits - x.edits ||
          x.name.localeCompare(y.name),
      )
      .slice(0, KNOWS_SHOWN);

    const machines: Machine[] = [...this.byHost(a.remote_paths).entries()]
      .map(([host, use]) => ({ host, ...use }))
      .sort((x, y) => y.writes + y.reads - (x.writes + x.reads))
      .slice(0, MACHINES_SHOWN);

    const total = (m: Record<string, number>) => Object.values(m).reduce((n, v) => n + v, 0);
    const shellTotal = [...shell.values()].reduce(
      (t, u) => ({
        reads: t.reads + u.reads,
        writes: t.writes + u.writes,
        maybeReads: t.maybeReads + u.maybeReads,
        maybeWrites: t.maybeWrites + u.maybeWrites,
      }),
      NOTHING(),
    );
    const committed = Object.values(a.commit_lines ?? {}).reduce(
      (t, l) => ({ added: t.added + l.added, deleted: t.deleted + l.deleted }),
      { added: 0, deleted: 0 },
    );
    return {
      name: a.name,
      // A session that was never named keeps its id — 36 characters of hex,
      // which is worth saying out loud rather than letting it read as a name.
      anonymous: /^[0-9a-f]{8}-[0-9a-f]{4}-/.test(a.name),
      reads: total(a.reads) + shellTotal.reads,
      writes: total(a.writes) + shellTotal.writes,
      shellReads: shellTotal.reads,
      shellWrites: shellTotal.writes,
      maybeReads: shellTotal.maybeReads,
      maybeWrites: shellTotal.maybeWrites,
      added: committed.added,
      deleted: committed.deleted,
      commits: a.commits ?? 0,
      delegated: a.delegated,
      first: a.first,
      last: a.last,
      places,
      machines,
      knows,
    };
  }

  /** Path-keyed uses, folded onto the project each path sits in. */
  private byProject(paths: Record<string, MemoryUse> | undefined) {
    const out = new Map<string, Counted>();
    for (const [path, use] of Object.entries(paths ?? {})) {
      const project = projectOf(path);
      if (!project) continue;
      const at = out.get(project) ?? NOTHING();
      at.reads += use.reads;
      at.writes += use.edits;
      // Kept in their own two fields rather than added to the two above: the
      // artefact separates them because they are a different kind of evidence,
      // and folding them here would undo that at the last step.
      at.maybeReads += use.maybe_reads ?? 0;
      at.maybeWrites += use.maybe_edits ?? 0;
      out.set(project, at);
    }
    return out;
  }

  /** Committed lines, folded the same way. */
  private linesByProject(a: Agent) {
    const out = new Map<string, { added: number; deleted: number }>();
    for (const [path, delta] of Object.entries(a.commit_lines ?? {})) {
      const project = projectOf(path);
      if (!project) continue;
      const at = out.get(project) ?? { added: 0, deleted: 0 };
      at.added += delta.added;
      at.deleted += delta.deleted;
      out.set(project, at);
    }
    return out;
  }

  /** Remote uses are keyed `host:/absolute/path`; the machine is the head. */
  private byHost(paths: Record<string, MemoryUse> | undefined) {
    const out = new Map<string, { reads: number; writes: number }>();
    for (const [key, use] of Object.entries(paths ?? {})) {
      const host = key.slice(0, key.indexOf(':'));
      if (!host) continue;
      const at = out.get(host) ?? { reads: 0, writes: 0 };
      at.reads += use.reads;
      at.writes += use.edits;
      out.set(host, at);
    }
    return out;
  }
}
