import { Component, computed, inject, signal } from '@angular/core';
import { MAT_BOTTOM_SHEET_DATA } from '@angular/material/bottom-sheet';
import { MatButtonModule } from '@angular/material/button';
import { MatButtonToggleModule } from '@angular/material/button-toggle';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';

import { ConsoleApi } from './console-api';
import { Task } from './models';
import { Rendered } from './rendered';
import { reason } from './errors';

/** What the sheet is opened with. */
export interface Which {
  readonly session: string;
  readonly name?: string;
}

/** Where one status stands: how it reads, where it sorts, whether it is work. */
export interface Standing {
  readonly rank: number;
  readonly title: string;
  readonly icon: string;
  /** Still work in hand. Mirrors the service's `Status::is_open`. */
  readonly open: boolean;
}

/**
 * How the statuses sort, and how they read.
 *
 * ⚠ **Underway above merely open.** The middle state is the answer to "what is
 * this session actually on", which is the question this sheet is opened with.
 * The closed ones sort last and are hidden by default: three hundred finished
 * things above eight open ones is a list nobody scrolls.
 *
 * ⚠ **`open` is a field rather than `status !== 'done'`.** The service grew a
 * fourth state, `dropped` — closed without being done — and the filter here said
 * "not done", which is the same thing right up until it isn't. Five dropped
 * tasks then showed among the open ones on the tasks session, with a question
 * mark for an icon, and the toggle offered to reveal work it was already
 * showing. The service made exactly this mistake first and fixed it the same
 * way: `Status::is_open` is a method there precisely so a fifth state cannot
 * quietly leave one call site behind.
 */
const STATUS: Record<string, Standing> = {
  doing: { rank: 0, title: 'underway', icon: 'pending', open: true },
  open: { rank: 1, title: 'open', icon: 'radio_button_unchecked', open: true },
  done: { rank: 2, title: 'done', icon: 'check_circle', open: false },
  // Not the primary colour the done mark gets, and not a tick: dropped is
  // "decided against", and a list that credited it as finished work would be
  // the reason the service keeps the two apart at all. The OUTLINE cross rather
  // than the filled `cancel` — that one renders as a solid disc, which made the
  // least important row on the screen the loudest mark on it.
  dropped: { rank: 3, title: 'dropped', icon: 'highlight_off', open: false },
};

/** Anything the CLI grows later sorts with the open ones rather than vanishing:
 *  a state this console has not heard of is news, not a reason to hide a row. */
const UNKNOWN: Standing = { rank: 1, title: 'open', icon: 'help', open: true };

/** Where a status stands, including one this console has never heard of. */
export function standingOf(status: string): Standing {
  return STATUS[status] ?? UNKNOWN;
}

/** The rows to draw: open work first, and the closed ones only when asked. */
export function shownTasks(all: readonly Task[], everything: boolean): Task[] {
  const wanted = everything ? [...all] : all.filter((task) => standingOf(task.status).open);
  // Stable within a status: the list is already in the order the session made
  // them, and the sort only lifts what is underway to the top.
  return wanted.sort((left, right) => standingOf(left.status).rank - standingOf(right.status).rank);
}

/**
 * What the "All" toggle would reveal, in the service's own words — empty when it
 * would reveal nothing and the toggle should not be drawn at all.
 *
 * The two closed states are named separately while only one of them is present,
 * because "13 done" and "5 dropped" are different facts and both fit. Together
 * they collapse to a count: "13 done, 5 dropped" is a long label on a phone, and
 * the icons in the list already tell them apart once the toggle is on.
 */
export function closedLabel(all: readonly Task[]): string {
  const closed = all.filter((task) => !standingOf(task.status).open);
  const dropped = closed.filter((task) => task.status === 'dropped').length;
  const done = closed.length - dropped;
  if (done > 0 && dropped > 0) return `${closed.length} closed`;
  if (dropped > 0) return `${dropped} dropped`;
  return done > 0 ? `${done} done` : '';
}

/**
 * A session's own task list, read-only.
 *
 * ⚠ **Read-only on purpose, and it should stay that way.** These are written by
 * the session's task tools; a second surface editing them is how one list
 * becomes two that disagree — which is the defect this console has now fixed
 * three times in other places. What this adds is the thing the session cannot
 * do: show you all of it at once, months after the fact.
 */
@Component({
  selector: 'app-tasks-sheet',
  templateUrl: './tasks-sheet.html',
  styleUrl: './tasks-sheet.scss',
  imports: [MatButtonModule, MatButtonToggleModule, MatIconModule, MatProgressBarModule, Rendered],
})
export class TasksSheet {
  private api = inject(ConsoleApi);
  protected readonly which = inject<Which>(MAT_BOTTOM_SHEET_DATA);

  protected readonly all = signal<Task[] | undefined>(undefined);
  /**
   * Why the list could not be read, when it could not be.
   *
   * dev-lint: allow-sticky-error the sheet reads the list once, on the way up,
   * and offers no retry — so there is no later success for this to be stale
   * against. Withdrawing it means closing the sheet, which destroys this
   * component and the message with it. The per-task failures below DO retry and
   * are cleared accordingly.
   */
  protected readonly trouble = signal('');
  /** Whether finished tasks are shown. Off by default — see [STATUS]. */
  protected readonly everything = signal(false);
  /** The task whose prose is open, and what it said. One at a time: this is a
   *  sheet on a phone, and two expanded write-ups are no longer a list. */
  protected readonly opened = signal<string | undefined>(undefined);
  protected readonly said = signal<Record<string, string>>({});
  /**
   * Why one task's write-up could not be fetched, per task.
   *
   * ⚠ **Separate from [said], because a failure kept as the text would be
   * permanent.** Written into the same map, the message became the task's
   * description as far as this sheet was concerned — folding the row and opening
   * it again found something cached and never asked twice, so a request that
   * failed once failed for the life of the sheet. A fresh attempt withdraws it.
   */
  protected readonly failed = signal<Record<string, string>>({});

  protected readonly shown = computed(() => shownTasks(this.all() ?? [], this.everything()));
  /** Said plainly rather than as a count of nothing: an empty list and a list
   *  with nothing left open are different facts about a session. */
  protected readonly empty = computed(() => (this.all() ?? []).length === 0);
  /** What the toggle offers to reveal, and whether there is anything to. */
  protected readonly closed = computed(() => closedLabel(this.all() ?? []));

  constructor() {
    this.api.tasks(this.which.session).subscribe({
      next: (tasks) => this.all.set(tasks),
      error: (failure: unknown) => {
        this.all.set([]);
        this.trouble.set(reason(failure));
      },
    });
  }

  protected standingOf(task: Task): Standing {
    return standingOf(task.status);
  }

  /** Open a task's write-up, or fold it away again. Fetched once and kept. */
  protected open(task: Task): void {
    if (!task.detailed) return;
    if (this.opened() === task.id) {
      this.opened.set(undefined);
      return;
    }
    this.opened.set(task.id);
    if (this.said()[task.id] !== undefined) return;
    // Opening it again is the retry, so the last attempt's failure goes first.
    this.failed.update((held) =>
      Object.fromEntries(Object.entries(held).filter(([id]) => id !== task.id)),
    );
    this.api.task(this.which.session, task.id).subscribe({
      next: ({ description }) => this.said.update((held) => ({ ...held, [task.id]: description })),
      error: (failure: unknown) =>
        this.failed.update((held) => ({ ...held, [task.id]: reason(failure) })),
    });
  }
}
