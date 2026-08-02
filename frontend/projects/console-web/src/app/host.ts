import { HttpErrorResponse, HttpInterceptorFn } from '@angular/common/http';
import { catchError, throwError } from 'rxjs';

/**
 * The Android wrapper, when this page is running inside it.
 *
 * The one thing the page cannot do for itself. Requests are signed with a key in
 * this phone's StrongBox that stays usable for five minutes after an
 * authentication, and when that window closes the TLS handshake is refused
 * before any request leaves the device. Chromium caches its client-certificate
 * decision per host and reuses the key handle without asking the app again, so
 * `onReceivedClientCertRequest` never runs and nothing prompts — the page simply
 * stops being answered, with no way to say why.
 *
 * Undefined in a browser, which is not a degraded mode: on the desk the console
 * is reached over loopback with no certificate involved, so there is nothing to
 * renew and nothing to ask.
 */
interface ConsoleHost {
  /**
   * Ask for the signing key to be put back inside its window.
   *
   * Fire and forget. The prompt is the app's to run and the person's to answer,
   * and the poll already in flight is what notices the answer — so there is no
   * callback to get wrong, and a refusal costs nothing but the next attempt.
   */
  renew(): void;
}

declare global {
  interface Window {
    consoleHost?: ConsoleHost;
  }
}

/**
 * When nothing answers, ask the phone whether it is holding the key back.
 *
 * ⚠ Status 0 is not evidence of an unreachable server — it is the *absence* of
 * evidence, and a refused client certificate produces exactly the same nothing.
 * That ambiguity is what makes this an interceptor rather than a message: the
 * page cannot tell the two apart, so it asks the only party that can and lets
 * the poll settle it.
 *
 * Safe to call when the key is fine: the app checks before prompting, so an
 * ordinary outage — a sleeping Mac, a dropped tunnel — asks the person for
 * nothing.
 */
export const renewOnRefusal: HttpInterceptorFn = (req, next) =>
  next(req).pipe(
    catchError((err: unknown) => {
      if (err instanceof HttpErrorResponse && err.status === 0) {
        window.consoleHost?.renew();
      }
      return throwError(() => err);
    }),
  );
