import { Component, computed, inject, signal } from '@angular/core';
import { MAT_BOTTOM_SHEET_DATA } from '@angular/material/bottom-sheet';
import { MatButtonModule } from '@angular/material/button';
import { MatButtonToggleModule } from '@angular/material/button-toggle';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';

import { ConsoleApi } from './console-api';
import { Task } from './models';
import { reason } from './errors';

/** What the sheet is opened with. */
export interface Which {
  readonly session: string;
  readonly name?: string;
}

/**
 * How the statuses sort, and how they read.
 *
 * ⚠ **Underway above merely open.** The CLI keeps three states and the middle
 * one is the answer to "what is this session actually on", which is the question
 * this sheet is opened with. Completed sorts last and is hidden by default:
 * three hundred done things above eight open ones is a list nobody scrolls.
 */
const STATUS: Record<string, { rank: number; title: string; icon: string }> = {
  in_progress: { rank: 0, title: 'underway', icon: 'pending' },
  pending: { rank: 1, title: 'open', icon: 'radio_button_unchecked' },
  completed: { rank: 2, title: 'done', icon: 'check_circle' },
};

/** Anything the CLI grows later sorts with the open ones rather than vanishing:
 *  a state this console has not heard of is news, not a reason to hide a row. */
const UNKNOWN = { rank: 1, title: 'open', icon: 'help' };

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
  imports: [MatButtonModule, MatButtonToggleModule, MatIconModule, MatProgressBarModule],
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

  protected readonly shown = computed(() => {
    const all = this.all() ?? [];
    const wanted = this.everything() ? all : all.filter((task) => task.status !== 'completed');
    // Stable within a status: the list is already in the order the session made
    // them, and the sort only lifts what is underway to the top.
    return [...wanted].sort((left, right) => rankOf(left) - rankOf(right));
  });
  /** Said plainly rather than as a count of nothing: an empty list and a list
   *  with nothing left open are different facts about a session. */
  protected readonly empty = computed(() => (this.all() ?? []).length === 0);
  protected readonly done = computed(
    () => (this.all() ?? []).filter((task) => task.status === 'completed').length,
  );

  constructor() {
    this.api.tasks(this.which.session).subscribe({
      next: (tasks) => this.all.set(tasks),
      error: (failure: unknown) => {
        this.all.set([]);
        this.trouble.set(reason(failure));
      },
    });
  }

  protected statusOf(task: Task): { title: string; icon: string } {
    return STATUS[task.status] ?? UNKNOWN;
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

function rankOf(task: Task): number {
  return (STATUS[task.status] ?? UNKNOWN).rank;
}
