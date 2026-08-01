import { beforeEach, describe, expect, it } from 'vitest';
import { provideZonelessChangeDetection } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { provideRouter } from '@angular/router';

import { SearchView } from './search-view';
import { WorkMatch } from './models';
import { routes } from './app.routes';

const WORKERS: WorkMatch[] = [
  {
    name: 'hardware',
    edits: 35,
    reads: 1,
    files: [
      { path: 'xinutec-infra/plan/types.dhall', reads: 0, edits: 7 },
      { path: 'xinutec-infra/plan/deploy.dhall', reads: 1, edits: 2 },
    ],
  },
  { name: 'home', edits: 25, reads: 2, files: [{ path: 'k/dhall/home.dhall', reads: 2, edits: 25 }] },
];

describe('SearchView — who works on this', () => {
  let fixture: ComponentFixture<SearchView>;
  let http: HttpTestingController;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [SearchView],
      providers: [
        provideZonelessChangeDetection(),
        provideHttpClient(),
        provideHttpClientTesting(),
        provideRouter(routes),
      ],
    }).compileComponents();
    fixture = TestBed.createComponent(SearchView);
    http = TestBed.inject(HttpTestingController);
  });

  /** Run a query, answering the memory search and the work search in turn. */
  async function search(workers: WorkMatch[] | 'forbidden'): Promise<HTMLElement> {
    fixture.componentRef.instance.query.set('dhall');
    fixture.componentRef.instance.submit();
    await fixture.whenStable();
    http.expectOne((r) => r.url === '/api/search').flush({ hits: [], relaxed: false });
    const work = http.expectOne((r) => r.url === '/api/work');
    if (workers === 'forbidden') {
      work.flush({ error: 'forbidden' }, { status: 403, statusText: 'Forbidden' });
    } else {
      work.flush(workers);
    }
    await fixture.whenStable();
    return fixture.nativeElement as HTMLElement;
  }

  it('ranks the agents by changes and keeps reads beside, not inside', async () => {
    const el = await search(WORKERS);
    const names = [...el.querySelectorAll('.workers .who')].map((n) => n.textContent?.trim());
    expect(names).toEqual(['hardware', 'home']);
    const first = el.querySelector('.workers .counts')?.textContent ?? '';
    expect(first).toContain('35 changed');
    expect(first).toContain('1 read');
  });

  it('keeps the evidence collapsed until asked, then shows the paths', async () => {
    const el = await search(WORKERS);
    expect(el.querySelector('.workers .file-list')).toBeNull();

    el.querySelector<HTMLButtonElement>('.workers .worker')!.click();
    await fixture.whenStable();

    const paths = [...el.querySelectorAll('.workers .file-list code')].map((n) => n.textContent);
    expect(paths).toEqual(['xinutec-infra/plan/types.dhall', 'xinutec-infra/plan/deploy.dhall']);
  });

  it('shows no panel to a share-link recipient, and no error either', async () => {
    // 403 is the intended answer for a share token, not a failure: the roster is
    // owner-only. It must read as "not yours to see", never as a broken search.
    const el = await search('forbidden');
    expect(el.querySelector('.workers')).toBeNull();
    expect(el.textContent).not.toContain('Who works on this');
  });
});
