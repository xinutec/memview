import { Component, DestroyRef, computed, inject, signal } from '@angular/core';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { MatButtonModule } from '@angular/material/button';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { catchError, of } from 'rxjs';

import { MemviewApi } from './memview-api';
import { Did, Effect, Evidence, Moment, Timeline } from './models';

/**
 * The wire's one-letter verbs, as a reader reads them.
 *
 * ⚠ **The wire is `r`/`w`/`s`/`u` and this page must not show that.** The
 * artefact renames every variant to a character because it is hundreds of
 * thousands of rows read over a VPN; that is the artefact's business, not the
 * reader's. Drawn straight, a turn read `w` and `s`.
 */
const DID: Record<Did, string> = {
  r: 'read',
  w: 'wrote',
  // Not "read": consulting a file and searching in it are different questions,
  // and the artefact keeps them apart on purpose.
  s: 'searched',
  u: 'unnamed',
};

/** A row's evidence, as the page holds it while the request is in flight. */
interface Opened {
  /** The key the row was opened by, so a stale answer can be recognised. */
  agent: string;
  at: number;
  /** `undefined` until the answer arrives — NOT an empty list. */
  evidence?: Evidence;
  failed: boolean;
}

/** Filters the page offers, all optional and all composing. */
interface Filter {
  agent?: string;
  project?: string;
  kind?: string;
}

/** Kinds listed in the shape strip. Beyond this the tail is one-offs. */
const KINDS_SHOWN = 8;

/**
 * What the sessions have been doing, minute by minute, newest first.
 *
 * The roster next door says where an agent works; this says what it was doing
 * and how it turned out. A row is one session's one minute of one kind of work,
 * folded — `n` is how many activities went into it.
 *
 * **A turn opens onto its evidence, and that is the whole point of the page.**
 * `/api/doing` is derived throughout: no command line, no prompt, no output.
 * Standing on a row, the question is *which files, and how do you know?* —
 * which is `/api/effects`, keyed by the `(agent, at)` the row already carries.
 * Without that, a timeline is a list of assertions with nothing under them.
 *
 * ⚠ **What could not be named is drawn, never dropped.** 7,305 effects in the
 * artefact have a subject nobody could resolve. A panel showing only what
 * resolved would read as a complete account of the turn, and it is not one.
 * Same rule the parse sheet follows on the phone: say what you could not read.
 *
 * ⚠ **The summary is of the whole filtered range, not of the page.** Two
 * hundred rows cannot show the shape of two hundred thousand, so the server
 * counts every kind that matched and this draws that beside the rows. Reading
 * the strip off the visible page instead would quietly describe a sample.
 */
@Component({
  selector: 'app-timeline-view',
  imports: [MatButtonModule, MatIconModule, MatProgressBarModule],
  templateUrl: './timeline-view.html',
  styleUrl: './timeline-view.scss',
})
export class TimelineView {
  private api = inject(MemviewApi);
  /**
   * ⚠ **Held, not reached for at the call site.** `takeUntilDestroyed()` with no
   * argument must run in an injection context, which a click handler is not —
   * it threw there, after `loading` had already been set, so the page hung on a
   * progress bar that could never resolve. The constructor's first load hid it:
   * that one IS in context. Injecting the ref once makes both paths the same.
   */
  private destroyRef = inject(DestroyRef);

  readonly timeline = signal<Timeline | undefined>(undefined);
  readonly loading = signal(true);
  /** Owner-only behind the API; a share token gets 403 and this says so. */
  readonly denied = signal(false);
  readonly filter = signal<Filter>({});
  readonly opened = signal<Opened | undefined>(undefined);

  constructor() {
    this.load();
  }

  private load(): void {
    this.loading.set(true);
    // ⚠ Withdrawn before every attempt. Set once and never cleared, a 403 from
    // one load would keep saying "owner-only" over the next load's real data.
    this.denied.set(false);
    this.api
      .doing(this.filter())
      .pipe(
        catchError(() => {
          this.denied.set(true);
          return of(undefined);
        }),
        takeUntilDestroyed(this.destroyRef),
      )
      .subscribe((page) => {
        this.timeline.set(page);
        this.loading.set(false);
      });
  }

  /** The kinds worth a chip, biggest first. */
  readonly kinds = computed(() => (this.timeline()?.summary ?? []).slice(0, KINDS_SHOWN));

  /** The share of the filtered range this page is actually showing. */
  readonly shown = computed(() => this.timeline()?.moments.length ?? 0);

  /**
   * How much of the range failed, as a share.
   *
   * Of the WHOLE filtered range rather than the page, for the reason the
   * summary is: a run of failures at the top would otherwise read as the
   * fleet's failure rate.
   */
  readonly failedShare = computed(() => {
    const page = this.timeline();
    if (!page || page.total === 0) return 0;
    return page.failed / page.total;
  });

  /** Filter by one facet, keeping the others. Clicking the active one clears it. */
  narrow(key: keyof Filter, value: string): void {
    const now = this.filter();
    this.filter.set({ ...now, [key]: now[key] === value ? undefined : value });
    this.opened.set(undefined);
    this.load();
  }

  clear(): void {
    this.filter.set({});
    this.opened.set(undefined);
    this.load();
  }

  readonly filtered = computed(() => Object.values(this.filter()).some((v) => v !== undefined));

  /** Whether this row is the one currently open. */
  isOpen(moment: Moment): boolean {
    const open = this.opened();
    return !!open && open.agent === moment.agent && open.at === moment.at;
  }

  /**
   * Open a row onto its evidence, or close it if it is already open.
   *
   * ⚠ **The pending state is `evidence: undefined`, and a template must not
   * treat it as an empty list.** "Nothing to show" and "nothing yet" are
   * different sentences, and drawing the first while the request is in flight
   * is a claim the page cannot support.
   */
  open(moment: Moment): void {
    if (this.isOpen(moment)) {
      this.opened.set(undefined);
      return;
    }
    const key = { agent: moment.agent, at: moment.at };
    this.opened.set({ ...key, failed: false });
    this.api
      .effects(moment.agent, moment.at)
      .pipe(
        catchError(() => of(undefined)),
        takeUntilDestroyed(this.destroyRef),
      )
      .subscribe((evidence) => {
        // ⚠ The answer to a row that is no longer open is dropped. Two quick
        // taps race, and the slower reply would otherwise land under the wrong
        // row — evidence attributed to a turn that did not do it.
        const open = this.opened();
        if (open?.agent !== key.agent || open.at !== key.at) return;
        this.opened.set({ ...open, evidence, failed: evidence === undefined });
      });
  }

  /** The minute a row happened, as a local time. */
  when(at: number): string {
    return new Date(at * 60_000).toLocaleString(undefined, {
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    });
  }

  /** What an effect did, in a word rather than in the wire's letter. */
  did(effect: Effect): string {
    return DID[effect.did] ?? effect.did;
  }

  /** What it did it to, or the pattern that bounds it, or neither. */
  subject(effect: Effect): string {
    return effect.path ?? effect.pattern ?? 'could not be named';
  }
}
