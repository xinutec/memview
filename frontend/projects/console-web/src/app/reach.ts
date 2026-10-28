import { Injectable, inject, signal } from '@angular/core';
import { firstValueFrom } from 'rxjs';

import { ConsoleApi } from './console-api';
import { Local } from './local';
import type { Unreached } from './models';

const KEY = 'unreached';

/** The stretch under way: no answer since `first_failure`. */
export interface Stretch {
  readonly last_answer?: number;
  readonly first_failure: number;
  readonly last_failure: number;
  readonly failures: number;
}

/**
 * Stretches when the runner did not answer, as this client saw them: from its
 * last answer to its first answer again. Measured here because nothing reaches
 * the runner to measure, and handed over once it answers — `console/src/reach.rs`.
 * Kept in storage until then, so a client closed mid-stretch still reports it;
 * its end then lies somewhere between its last failure and the answer.
 */
@Injectable({ providedIn: 'root' })
export class Reach {
  private api = inject(ConsoleApi);
  private store = inject(Local);
  private lastAnswer?: number;
  private closed: readonly Unreached[] = [];
  private reporting = false;
  private readonly loaded = this.restore();

  /** The stretch under way, or nothing while the runner answers. */
  readonly stretch = signal<Stretch | undefined>(undefined);

  /** The runner answered at `now`: a stretch under way ends, and is reported. */
  async answered(now = Date.now()): Promise<void> {
    await this.loaded;
    this.lastAnswer = now;
    const open = this.stretch();
    if (open) {
      this.stretch.set(undefined);
      this.closed = [...this.closed, { ...open, answered_again: now }];
      await this.save();
    }
    if (this.closed.length) await this.report();
  }

  /** No answer at `now`: a stretch begins, or goes on. */
  async unanswered(now = Date.now()): Promise<void> {
    await this.loaded;
    const open = this.stretch();
    this.stretch.set(
      open
        ? { ...open, last_failure: now, failures: open.failures + 1 }
        : {
            ...(this.lastAnswer !== undefined && { last_answer: this.lastAnswer }),
            first_failure: now,
            last_failure: now,
            failures: 1,
          },
    );
    await this.save();
  }

  /** Hand the ended stretches over; kept for the next answer if this fails. */
  private async report(): Promise<void> {
    if (this.reporting) return;
    this.reporting = true;
    const sending = this.closed;
    try {
      await firstValueFrom(this.api.reach(sending), { defaultValue: undefined });
      const sent = new Set(sending.map((it) => it.first_failure));
      this.closed = this.closed.filter((it) => !sent.has(it.first_failure));
      await this.save();
    } catch {
      // Kept: the runner keeps a stretch once however often it is told.
    } finally {
      this.reporting = false;
    }
  }

  private async save(): Promise<void> {
    await this.store.set(KEY, { stretch: this.stretch(), closed: this.closed });
  }

  /** What an earlier run kept, checked: storage outlives the build that wrote it. */
  private async restore(): Promise<void> {
    const kept: unknown = await this.store.get(KEY);
    if (typeof kept !== 'object' || kept === null) return;
    if ('stretch' in kept && isStretch(kept.stretch)) this.stretch.set(kept.stretch);
    if ('closed' in kept && Array.isArray(kept.closed)) {
      const closed: unknown[] = kept.closed;
      this.closed = closed.filter(
        (one): one is Unreached =>
          isStretch(one) && 'answered_again' in one && typeof one.answered_again === 'number',
      );
    }
  }
}

function isStretch(value: unknown): value is Stretch {
  return (
    typeof value === 'object' &&
    value !== null &&
    'first_failure' in value &&
    typeof value.first_failure === 'number' &&
    'last_failure' in value &&
    typeof value.last_failure === 'number' &&
    'failures' in value &&
    typeof value.failures === 'number'
  );
}
