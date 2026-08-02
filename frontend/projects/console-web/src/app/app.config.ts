import {
  ApplicationConfig,
  provideBrowserGlobalErrorListeners,
  provideZonelessChangeDetection,
} from '@angular/core';
import { provideHttpClient, withFetch, withInterceptors } from '@angular/common/http';
import { provideRouter, withComponentInputBinding } from '@angular/router';

import { routes } from './app.routes';
import { renewOnRefusal } from './host';

export const appConfig: ApplicationConfig = {
  providers: [
    provideBrowserGlobalErrorListeners(),
    provideZonelessChangeDetection(),
    // Every request, because the failure it handles belongs to the transport
    // rather than to any one call — see [[renewOnRefusal]].
    provideHttpClient(withFetch(), withInterceptors([renewOnRefusal])),
    // Route params bind to component inputs (:id → SessionView.id), so the URL
    // is the source of truth for which session is open — which is what makes a
    // session linkable from memview's /agents page later.
    provideRouter(routes, withComponentInputBinding()),
  ],
};
