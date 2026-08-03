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
    // Status 0 is not a server saying no — it is no answer at all, and the two
    // things that produce it are indistinguishable from here.
    //
    // ⚠ It said "the Mac is not answering", and was wrong in the case it hit
    // most: the Mac was answering, and this phone's key had passed the end of
    // its authentication window, so the TLS handshake was refused before any
    // request left the device. Chromium caches its client-certificate decision
    // per host and reuses the key without asking the app again, so nothing
    // prompts and every request fails in silence. Naming only the far cause sent
    // us looking at the Mac, the tunnel and isis while the answer was in the
    // phone's own log.
    if (err.status === 0)
      return 'no answer — the Mac may be asleep, or this phone may need unlocking';
    return `the runner answered ${err.status}`;
  }
  if (err instanceof Error) return err.message;
  return 'something went wrong';
}
