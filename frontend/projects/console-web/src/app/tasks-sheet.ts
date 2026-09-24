import { Component, computed, inject, signal } from '@angular/core';
import { MAT_BOTTOM_SHEET_DATA } from '@angular/material/bottom-sheet';
import { MatButtonModule } from '@angular/material/button';
import { MatButtonToggleModule } from '@angular/material/button-toggle';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';

import { ConsoleApi } from './console-api';
import { Status, Task } from './models';
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
 * How the statuses sort, and how they read. Underway above merely open — the
 * answer to "what is this session actually on"; the closed ones last and
 * hidden by default. `open` is a field rather than `status !== 'done'`: the
 * service grew `dropped`, closed without being done, and "not done" showed five
 * of them among the open work. `Status::is_open` is a method there for the same
 * reason.
 */
const STATUS: Record<Exclude<Status, { unknown: string }>, Standing> = {
  doing: { rank: 0, title: 'underway', icon: 'pending', open: true },
  open: { rank: 1, title: 'open', icon: 'radio_button_unchecked', open: true },
  done: { rank: 2, title: 'done', icon: 'check_circle', open: false },
  // Not the done mark's colour and not a tick: dropped is "decided against". The
  // OUTLINE cross — the filled `cancel` renders as a solid disc, the loudest mark
  // on the screen for the least important row.
  dropped: { rank: 3, title: 'dropped', icon: 'highlight_off', open: false },
};

/**
 * Anything the service grows later sorts with the open ones rather than
 * vanishing: a state this console has not heard of is news.
 */
const UNKNOWN: Standing = { rank: 1, title: 'open', icon: 'help', open: true };

/** Where a status stands, including one this console has never heard of. */
export function standingOf(status: Status): Standing {
  return typeof status === 'string' ? STATUS[status] : UNKNOWN;
}

/** The rows to draw: open work first, and the closed ones only when asked. */
export function shownTasks(all: readonly Task[], everything: boolean): Task[] {
  const wanted = everything ? [...all] : all.filter((task) => standingOf(task.status).open);
  // Stable within a status: the list is already in the session's order.
  return wanted.sort((left, right) => standingOf(left.status).rank - standingOf(right.status).rank);
}

/**
 * Whether a rank lifts a task above the unranked work. `P0` and `P1` only: `P2`
 * is where an unranked task already sits, and `P3`/`P4` sort below it. A level
 * this console has never heard of is drawn quietly as its own letters.
 */
export function above(priority: string | undefined): boolean {
  return priority === 'P0' || priority === 'P1';
}

/**
 * What a deadline says, spelled out for the label rather than the row: the row
 * gets an icon, since the date pushed a long subject from 9 wrapped lines to
 * 12 at phone width. Empty when there is no deadline, which is almost every task.
 */
export function dueLabel(task: Task): string {
  if (!task.due) return '';
  // `overdue` from the service, never `due < today` worked out here — see
  // [[Task.overdue]].
  return task.overdue ? `overdue — was due ${task.due}` : `due ${task.due}`;
}

/**
 * What a task is waiting for, by number — what a session calls a task in its
 * prose (`#418 done`). Empty unless the service says it is still blocked: the
 * link survives its blocker closing, as a record.
 */
export function waitingOn(task: Task): string {
  if (!task.blocked || !task.blocked_on?.length) return '';
  return `waiting on ${task.blocked_on.map((id) => `#${id}`).join(', ')}`;
}

/**
 * What the "All" toggle would reveal, in the service's own words — empty when
 * nothing. "13 done" and "5 dropped" are named separately while only one is
 * present; together they collapse to a count.
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
 * A session's own task list, read-only on purpose: a second surface editing
 * them is how one list becomes two that disagree. What this adds is seeing all
 * of it at once, months after the fact.
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
   * dev-lint: allow-sticky-error the sheet reads the list once and offers no
   * retry, so there is no later success for this to be stale against.
   */
  protected readonly trouble = signal('');
  /** Whether finished tasks are shown. Off by default — see [STATUS]. */
  protected readonly everything = signal(false);
  /**
   * The task whose prose is open, and what it said. One at a time: two expanded
   * write-ups on a phone are no longer a list.
   */
  protected readonly opened = signal<string | undefined>(undefined);
  protected readonly said = signal<Record<string, string>>({});
  /**
   * Why one task's write-up could not be fetched, per task. Separate from [said]:
   * kept as the text, a failure became the description for the life of the sheet.
   */
  protected readonly failed = signal<Record<string, string>>({});

  protected readonly shown = computed(() => shownTasks(this.all() ?? [], this.everything()));
  /**
   * Said plainly rather than as a count of nothing: an empty list and a list with
   * nothing left open are different facts.
   */
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

  protected above(task: Task): boolean {
    return above(task.priority);
  }

  protected dueLabel(task: Task): string {
    return dueLabel(task);
  }

  protected waitingOn(task: Task): string {
    return waitingOn(task);
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
