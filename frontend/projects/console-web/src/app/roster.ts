import { Injectable, inject, signal } from '@angular/core';

import { ConsoleApi } from './console-api';
import { reason } from './errors';
import type { Overview } from './models';
import { Reach } from './reach';
import { Updates } from './updates';

/** How often the runner is asked what it is holding. */
const EVERY_MS = 5000;

/**
 * What the runner is holding, polled once for the whole app.
 *
 * ⚠ **One poll, not one per view.** The list and the open conversation both want
 * this and both used to fetch it on their own five-second timer, each with its
 * own copy of the bundle check, the reachability patience and the wording of the
 * failure. Three facts in two places drift, and the timer is the kind that gets
 * left running — a poll per visit to the list once had the phone asking twenty
 * times over.
 *
 * ⚠ **A view READS this; it does not own it.** A request does not stop when the
 * page that made it does, so a per-view subscription can land after the view is
 * gone and put back what leaving was supposed to clear. Reading a signal cannot:
 * there is nothing to land into.
 */
@Injectable({ providedIn: 'root' })
export class Roster {
  private api = inject(ConsoleApi);
  private updates = inject(Updates);
  /** How patient the banner is. Its own, because the patience is about this
   *  poll and there is now exactly one. See [[Reach]]. */
  private reach = new Reach();

  /** The last answer, or nothing before the first one. */
  readonly state = signal<Overview | undefined>(undefined);

  /** Why the runner cannot be reached, once it has missed enough polls to be
   *  worth saying — see [[Reach]] for the patience. */
  readonly unreachable = signal('');

  private timer?: ReturnType<typeof setInterval>;
  private readers = 0;

  /**
   * Start reading, and stop when the returned function is called.
   *
   * Counted rather than a plain start/stop: both views are alive at once for the
   * moment a route change overlaps them, and the first to leave must not take
   * the poll away from the one arriving.
   */
  follow(): () => void {
    this.readers += 1;
    this.ask();
    this.timer ??= setInterval(() => this.ask(), EVERY_MS);
    let stopped = false;
    return () => {
      if (stopped) return;
      stopped = true;
      this.readers -= 1;
      if (this.readers > 0) return;
      clearInterval(this.timer);
      this.timer = undefined;
    };
  }

  /** Ask now — for the moment the app comes back to the front, when the poll
   *  has not been running. See [[Foreground]]. */
  ask(): void {
    this.api.state().subscribe({
      next: (state) => {
        this.state.set(state);
        this.updates.saw(state.bundle);
        this.unreachable.set(this.reach.answered());
      },
      error: (err: unknown) =>
        this.unreachable.set(this.reach.failed(`cannot reach the runner: ${reason(err)}`)),
    });
  }
}
