import {
  ApplicationConfig,
  provideBrowserGlobalErrorListeners,
  provideZonelessChangeDetection,
} from '@angular/core';
import { provideHttpClient, withFetch } from '@angular/common/http';
import { provideRouter, withComponentInputBinding } from '@angular/router';

import { routes } from './app.routes';

export const appConfig: ApplicationConfig = {
  providers: [
    provideBrowserGlobalErrorListeners(),
    provideZonelessChangeDetection(),
    provideHttpClient(withFetch()),
    // Route params bind to component inputs (:id → SessionView.id), so the URL
    // is the source of truth for which session is open — which is what makes a
    // session linkable from memview's /agents page later.
    provideRouter(routes, withComponentInputBinding()),
  ],
};
