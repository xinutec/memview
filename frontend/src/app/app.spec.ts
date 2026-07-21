import { beforeEach, describe, expect, it } from 'vitest';
import { provideZonelessChangeDetection } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { provideHttpClient, withInterceptors } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { provideRouter } from '@angular/router';

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
      ],
    }).compileComponents();
  });

  it('shows the shell once /api/me resolves', async () => {
    const fixture = TestBed.createComponent(App);
    await fixture.whenStable();
    const http = TestBed.inject(HttpTestingController);
    http
      .expectOne('/api/me')
      .flush({ user_id: 'local', display_name: 'Local', shared: false, auth_enabled: false });
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

  it('sends the stored share token on API requests', async () => {
    localStorage.setItem('memview_share_token', 'tok123');
    const fixture = TestBed.createComponent(App);
    await fixture.whenStable();
    const http = TestBed.inject(HttpTestingController);
    const req = http.expectOne('/api/me');
    expect(req.request.headers.get('X-Share-Token')).toBe('tok123');
    req.flush({ shared: true, auth_enabled: true });
  });
});
