import { HttpErrorResponse } from '@angular/common/http';

/**
 * What went wrong, in words fit to put on screen. One boundary: the console had
 * the error shape written out four times and one `String(err)` that put
 * `cannot reach the runner: [object Object]` in front of the user. Takes
 * `unknown` and narrows.
 *
 * A whole sentence, because it is drawn on its own or after a bold lead that
 * ends in a full stop — never glued to a prefix. The exception is the runner's
 * own plain-text answer, which is better than anything composed here and is
 * passed through in whatever words it used.
 */
export function reason(err: unknown): string {
  if (err instanceof HttpErrorResponse) {
    // The runner answers failures with a plain-text explanation, which is better
    // than anything composed here.
    if (typeof err.error === 'string' && err.error.trim()) return err.error.trim();
    // Status 0 is no answer at all, and two things produce it: this phone's key
    // past its authentication window (Chromium refuses the handshake silently), or
    // a Mac that is not answering. The phone is named first — measured over 3.9
    // days, the 235 status-0 episodes track waking hours, the opposite of a
    // sleeping Mac — and the Mac stays, being the case nobody would think of.
    if (err.status === 0) return 'This phone may need unlocking, or the Mac may be asleep.';
    return `The runner answered ${err.status}.`;
  }
  if (err instanceof Error) return err.message;
  return 'Something went wrong.';
}
