import { Injectable, inject, signal } from '@angular/core';

import { ConsoleApi } from './console-api';
import { reason } from './errors';
import type { Overview } from './models';
import { Patience } from './patience';
import { Updates } from './updates';

/** How often the runner is asked what it is holding. */
const EVERY_MS = 5000;

/**
 * What the runner is holding, polled once for the whole app. One poll, not one
 * per view: two timers drifted in three facts, and a poll per visit once had
 * the phone asking twenty times over. A view READS this; a per-view
 * subscription can land after the view is gone.
 */
@Injectable({ providedIn: 'root' })
export class Roster {
  private api = inject(ConsoleApi);
  private updates = inject(Updates);
  /** How patient the banner is. Its own, because there is exactly one poll. */
  private readonly patience = new Patience();

  /** The last answer, or nothing before the first one. */
  readonly state = signal<Overview | undefined>(undefined);

  /**
   * Why the runner cannot be reached, once it has been silent long enough to be
   * worth saying — see [[Patience]].
   */
  readonly notice = this.patience.worth;

  private timer?: ReturnType<typeof setInterval>;
  private readers = 0;

  /**
   * Start reading, and stop when the returned function is called. Counted: both
   * views are alive for the moment a route change overlaps them.
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

  /** Ask now — for the app coming back to the front. See [[Foreground]]. */
  ask(): void {
    this.api.state().subscribe({
      next: (state) => {
        this.state.set(state);
        this.updates.saw(state.bundle);
        this.patience.right();
      },
      error: (err: unknown) => this.patience.wrong({ kind: 'runner', what: reason(err) }),
    });
  }
}
