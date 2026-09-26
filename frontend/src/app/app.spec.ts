import { beforeEach, describe, expect, it } from 'vitest';
import { provideZonelessChangeDetection } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { provideHttpClient, withInterceptors } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { provideRouter } from '@angular/router';
import { provideServiceWorker } from '@angular/service-worker';

import { App } from './app';
import { authInterceptor } from './auth';
import { routes } from './app.routes';

describe('App', () => {
  beforeEach(async () => {
    localStorage.clear();
    await TestBed.configureTestingModule({
      imports: [App],
      providers: [
        provideZonelessChangeDetection(),
        provideHttpClient(withInterceptors([authInterceptor])),
        provideHttpClientTesting(),
        provideRouter(routes),
        // The shell starts the update check, so SwUpdate has to resolve here.
        // `enabled: false` gives the real class in its disabled state, which is
        // what a test runner is: no worker registered, no update to activate.
        provideServiceWorker('ngsw-worker.js', { enabled: false }),
      ],
    }).compileComponents();
  });

  it('shows the shell once /api/me resolves', async () => {
    const fixture = TestBed.createComponent(App);
    await fixture.whenStable();
    const http = TestBed.inject(HttpTestingController);
    http
      .expectOne('/api/me')
      .flush({ user_id: 'local', display_name: 'Local', auth_enabled: false });
    await fixture.whenStable();
    const el = fixture.nativeElement as HTMLElement;
    expect(el.querySelector('.brand')?.textContent).toContain('memory');
  });

  it('raises the sign-in wall on a 401 from /api/me', async () => {
    const fixture = TestBed.createComponent(App);
    await fixture.whenStable();
    const http = TestBed.inject(HttpTestingController);
    http
      .expectOne('/api/me')
      .flush({ error: 'not authenticated' }, { status: 401, statusText: 'Unauthorized' });
    await fixture.whenStable();
    const el = fixture.nativeElement as HTMLElement;
    expect(el.querySelector('.signin')).toBeTruthy();
    expect(el.querySelector('.signin a')?.getAttribute('href')).toContain('/login');
  });

  it('sends no share token, even one an old link left behind', async () => {
    localStorage.setItem('memview_share_token', 'tok123');
    const fixture = TestBed.createComponent(App);
    await fixture.whenStable();
    const http = TestBed.inject(HttpTestingController);
    const req = http.expectOne('/api/me');
    expect(req.request.headers.has('X-Share-Token')).toBe(false);
    req.flush({ user_id: 'local', display_name: 'Local', auth_enabled: false });
    localStorage.removeItem('memview_share_token');
  });
});
