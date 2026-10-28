import { HttpErrorResponse } from '@angular/common/http';

/**
 * What went wrong, in words fit to put on screen. One boundary, so no caller
 * writes out the error shape itself or puts `String(err)` — `[object Object]` —
 * in front of the user. Takes `unknown` and narrows.
 *
 * A whole sentence, because it is drawn on its own or after a bold lead that
 * ends in a full stop — never glued to a prefix. The exception is the runner's
 * own plain-text answer, passed through in whatever words it used.
 */
export function reason(err: unknown): string {
  if (err instanceof HttpErrorResponse) {
    // The runner answers failures with a plain-text explanation, which is better
    // than anything composed here.
    if (typeof err.error === 'string' && err.error.trim()) return err.error.trim();
    // Status 0 is no answer, and says nothing of where along the way it stopped:
    // the client's network or key, the way in, or the runner. Not guessed at.
    if (err.status === 0) return 'No answer from the runner.';
    return `The runner answered ${err.status}.`;
  }
  if (err instanceof Error) return err.message;
  return 'Something went wrong.';
}

/**
 * Why bytes asked for as a `Blob` did not arrive. With `responseType: 'blob'` the
 * console's sentence comes back as a `Blob` on `err.error`, where [[reason]]
 * finds no string, so it is read out here.
 */
export async function told(err: unknown): Promise<string> {
  const body: unknown = err && typeof err === 'object' ? Reflect.get(err, 'error') : undefined;
  if (body instanceof Blob) {
    const said = (await body.text()).trim();
    if (said) return said;
  }
  return reason(err);
}
