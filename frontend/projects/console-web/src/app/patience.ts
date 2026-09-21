import { Signal, signal } from '@angular/core';

import type { Notice } from './notice';

/**
 * How long trouble must last before the reader hears about it.
 *
 * ONE number for both things this console waits on, which is the whole point of
 * this file: a poll that has stopped answering used to wait two failures and a
 * stream that has stopped arriving used to wait eight seconds, so a reader
 * comparing the two banners was really comparing two clocks. Comfortably past
 * the browser's own reconnect, measured at about three seconds in the
 * phone-width harness, and past a burst of polls failing together while
 * `host.ts` renews the key.
 */
const WAIT_MS = 8000;

/**
 * Trouble worth saying: what to show once it has lasted [WAIT_MS], and nothing
 * while it may still be a blip.
 *
 * For a STATE. An action that fails says so on the press, with no patience at
 * all, because the reader is already looking at what they did.
 */
export class Patience {
  private readonly shown = signal<Notice | undefined>(undefined);
  private latest?: Notice;
  private timer?: ReturnType<typeof setTimeout>;

  /** What to say, or nothing. */
  readonly worth: Signal<Notice | undefined> = this.shown.asReadonly();

  /**
   * Whether anything is wrong at all, said or not.
   *
   * ⚠ **Not reactive, on purpose.** This is the raw state, which goes true for a
   * moment every time the browser reconnects; a marker drawn on it would blink at
   * a reader whose connection is fine. Drawing is [worth]'s job.
   */
  get troubled(): boolean {
    return this.latest !== undefined;
  }

  /**
   * Trouble, for the first time or again. The clock starts but never restarts:
   * trouble that keeps recurring is trouble that lasts.
   */
  wrong(notice: Notice): void {
    this.latest = notice;
    this.timer ??= setTimeout(() => this.shown.set(this.latest), WAIT_MS);
  }

  /** All well. Anything shown goes at once, and the clock stops. */
  right(): void {
    clearTimeout(this.timer);
    this.timer = undefined;
    this.latest = undefined;
    this.shown.set(undefined);
  }
}
