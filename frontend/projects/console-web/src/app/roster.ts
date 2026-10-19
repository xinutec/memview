import { Injectable, inject, signal } from '@angular/core';

import { ConsoleApi } from './console-api';
import { reason } from './errors';
import { Local } from './local';
import type { Overview } from './models';
import { Patience } from './patience';
import { Updates } from './updates';

/** How often the runner is asked what it is holding. */
const EVERY_MS = 5000;

/** How often the answer is kept for the next start, at most. */
const KEEP_EVERY_MS = 30_000;

/**
 * What the runner is holding, polled once for the whole app. One poll, not one
 * per view: two timers drift apart, and a poll per visit multiplies. A view reads
 * this; a per-view subscription can land after the view is gone.
 */
@Injectable({ providedIn: 'root' })
export class Roster {
  private api = inject(ConsoleApi);
  private updates = inject(Updates);
  /** How patient the banner is. Its own, because there is exactly one poll. */
  private readonly patience = new Patience();

  private store = inject(Local);
  private keptAt = 0;

  /** The last answer, or before the first one the answer this phone kept, if any. */
  readonly state = signal<Overview | undefined>(undefined);

  /** [state] is this phone's copy, and the runner has not answered since starting. */
  readonly kept = signal(false);

  /**
   * Why the runner cannot be reached, once it has been silent long enough to be
   * worth saying — see [[Patience]].
   */
  readonly notice = this.patience.worth;

  private timer?: ReturnType<typeof setInterval>;
  private readers = 0;

  constructor() {
    void this.revive();
  }

  /** The list as last seen, at once: the runner's answer replaces it. */
  private async revive(): Promise<void> {
    const stored: unknown = await this.store.get('kept-state');
    if (this.state() !== undefined || !drawable(stored)) return;
    this.state.set(stored);
    this.kept.set(true);
  }

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
        this.kept.set(false);
        if (Date.now() - this.keptAt > KEEP_EVERY_MS) {
          this.keptAt = Date.now();
          void this.store.set('kept-state', state);
        }
        this.updates.saw(state.bundle);
        this.patience.right();
      },
      error: (err: unknown) => this.patience.wrong({ kind: 'runner', what: reason(err) }),
    });
  }
}

/**
 * Whether a stored answer can be drawn. Checked as far as the list reads it,
 * not cast: storage outlives the build that wrote it.
 */
function drawable(value: unknown): value is Overview {
  if (typeof value !== 'object' || value === null || !('sessions' in value)) return false;
  const { sessions } = value;
  return (
    Array.isArray(sessions) &&
    sessions.every(
      (one: unknown) =>
        typeof one === 'object' && one !== null && 'id' in one && typeof one.id === 'string',
    )
  );
}
