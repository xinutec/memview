import { HttpErrorResponse, HttpInterceptorFn } from '@angular/common/http';
import { inject } from '@angular/core';
import { catchError, throwError } from 'rxjs';

import { Telemetry } from './telemetry';

/**
 * The Android wrapper, when this page is running inside it. Requests are signed
 * with a key in the phone's StrongBox usable for a set time after an
 * authentication; when that window closes the TLS handshake is refused before
 * any request leaves, and Chromium's cached client-certificate decision means
 * nothing prompts — the page simply stops being answered. Undefined in a
 * browser, where loopback needs no certificate.
 */
interface ConsoleHost {
  /**
   * Ask for the signing key to be put back inside its window. Fire and forget:
   * the poll already in flight is what notices the answer.
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
 * Status 0 is the absence of evidence, and a refused client certificate
 * produces the same nothing as an unreachable server; the app checks before
 * prompting, so an ordinary outage asks nobody. Every failure is traced on the
 * way past: without that, a failing app leaves a log that reads as a person
 * browsing contentedly.
 */
export const traceAndRenew: HttpInterceptorFn = (req, next) => {
  const telemetry = inject(Telemetry);
  return next(req).pipe(
    catchError((err: unknown) => {
      if (err instanceof HttpErrorResponse) {
        // The status is the label, and 0 is the interesting one: "no answer", the
        // failure that has no other record.
        telemetry.failure(req.url, err.status);
        if (err.status === 0) window.consoleHost?.renew();
      }
      return throwError(() => err);
    }),
  );
};
