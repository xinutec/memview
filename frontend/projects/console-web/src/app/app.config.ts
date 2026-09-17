import {
  ApplicationConfig,
  provideBrowserGlobalErrorListeners,
  provideZonelessChangeDetection,
} from '@angular/core';
import { provideHttpClient, withFetch, withInterceptors } from '@angular/common/http';
import { provideRouter, withComponentInputBinding } from '@angular/router';

import { routes } from './app.routes';
import { traceAndRenew } from './host';

export const appConfig: ApplicationConfig = {
  providers: [
    provideBrowserGlobalErrorListeners(),
    provideZonelessChangeDetection(),
    // Every request: the failure it handles belongs to the transport — see [[traceAndRenew]].
    provideHttpClient(withFetch(), withInterceptors([traceAndRenew])),
    // Route params bind to component inputs (`:id` → `SessionView.id`), so the URL
    // is the source of truth for which session is open.
    provideRouter(routes, withComponentInputBinding()),
  ],
};
