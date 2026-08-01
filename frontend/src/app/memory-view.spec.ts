import { beforeEach, describe, expect, it } from 'vitest';
import { provideZonelessChangeDetection } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { provideRouter } from '@angular/router';

import { MemoryView } from './memory-view';
import { MemoryPage } from './models';
import { routes } from './app.routes';

/** A page with only the fields the header reads; the rest is not under test. */
function page(extra: Partial<MemoryPage> = {}): MemoryPage {
  return {
    name: 'project_alpha',
    description: '',
    mtype: 'project',
    modified: null,
    html: '<p>Body.</p>',
    backlinks: [],
    outlinks: [],
    dangling: [],
    ...extra,
  };
}

describe('MemoryView origin', () => {
  let fixture: ComponentFixture<MemoryView>;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [MemoryView],
      providers: [
        provideZonelessChangeDetection(),
        provideHttpClient(),
        provideHttpClientTesting(),
        provideRouter(routes),
      ],
    }).compileComponents();
    fixture = TestBed.createComponent(MemoryView);
  });

  async function load(body: MemoryPage): Promise<HTMLElement> {
    fixture.componentRef.setInput('name', 'project_alpha');
    await fixture.whenStable();
    TestBed.inject(HttpTestingController).expectOne('/api/memory/project_alpha').flush(body);
    await fixture.whenStable();
    return fixture.nativeElement as HTMLElement;
  }

  it('names the agent that wrote the memory and links to the roster', async () => {
    const el = await load(page({ origin: { session: 's1', agent: 'hardware' } }));
    const link = el.querySelector('.origin a');
    expect(link?.textContent).toContain('hardware');
    expect(link?.getAttribute('href')).toBe('/agents');
  });

  it('says the session was pruned rather than showing a bare uuid', async () => {
    // The id is not useful to read, but it IS the only identifier left, so it
    // stays reachable on hover instead of being dropped.
    const el = await load(page({ origin: { session: '2a48acfa-738d-4346-bf9f-6c1fb0e74809' } }));
    const pruned = el.querySelector('.origin .pruned');
    expect(pruned?.textContent).toContain('since pruned');
    expect(pruned?.getAttribute('title')).toBe('2a48acfa-738d-4346-bf9f-6c1fb0e74809');
    expect(el.querySelector('.origin a')).toBeNull();
  });

  it('shows nothing at all when the server withheld the origin', async () => {
    // What a share-link recipient gets: the field is absent, and absence must
    // render as absence rather than as "written by undefined".
    const el = await load(page());
    expect(el.querySelector('.origin')).toBeNull();
    expect(el.textContent).not.toContain('written by');
  });
});
