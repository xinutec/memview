import { HttpErrorResponse, HttpInterceptorFn } from '@angular/common/http';
import { Injectable, signal } from '@angular/core';
import { inject } from '@angular/core';
import { catchError, throwError } from 'rxjs';

import { getShareToken } from './share-token';

/** Session state: who we are, and whether the sign-in wall is up. */
@Injectable({ providedIn: 'root' })
export class AuthStore {
  readonly needsSignIn = signal(false);
}

/**
 * Attaches the stored share token (if any) and flips the sign-in wall on any
 * 401 — the recall/messages pattern.
 */
export const authInterceptor: HttpInterceptorFn = (req, next) => {
  const auth = inject(AuthStore);
  const token = getShareToken();
  const withToken = token ? req.clone({ setHeaders: { 'X-Share-Token': token } }) : req;
  return next(withToken).pipe(
    catchError((err: unknown) => {
      if (err instanceof HttpErrorResponse && err.status === 401) {
        auth.needsSignIn.set(true);
      }
      return throwError(() => err);
    }),
  );
};
