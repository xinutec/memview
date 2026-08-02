import { HttpErrorResponse } from '@angular/common/http';

/**
 * What went wrong, in words fit to put on screen.
 *
 * One boundary rather than a shape declared at each callsite. The console had
 * `err: { error?: string }` written out four times and `String(err)` once, and
 * the one that stringified put **`cannot reach the runner: [object Object]`** in
 * front of the user — which says nothing, and says it in a way that looks like a
 * bug in the app rather than a Mac that is asleep.
 *
 * Nothing static caught that, and the two rules nearby explain why:
 * `DL-ANGULAR-STRINGIFIED-OBJECT` reads templates, not TypeScript, and exempts
 * `unknown` on purpose; `DL-ANGULAR-HTTP-ERROR-CLASSIFIED` fires on reading
 * `.status` off a *typed* `HttpErrorResponse`, and this read no field at all.
 *
 * Takes `unknown` and narrows, which is also what keeps it out of the second
 * rule's sights by construction.
 */
export function reason(err: unknown): string {
  if (err instanceof HttpErrorResponse) {
    // The runner answers failures with a plain-text explanation — "…is not
    // inside an allowed directory", "…looks like it is still in use" — and that
    // sentence is better than anything this function could compose.
    if (typeof err.error === 'string' && err.error.trim()) return err.error.trim();
    // Status 0 is not a server saying no; it is nothing answering at all, and on
    // a phone that nearly always means the Mac is asleep or the tunnel is down.
    if (err.status === 0) return 'the Mac is not answering';
    return `the runner answered ${err.status}`;
  }
  if (err instanceof Error) return err.message;
  return 'something went wrong';
}
