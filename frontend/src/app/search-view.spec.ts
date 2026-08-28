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
    added: 2126,
    deleted: 433,
    file_commits: 12,
    hosts: ['odin'],
    files: [
      // Renamed, and carrying the history of the name it had before — which
      // the row must say, or a file created last week appears to have a year of
      // work behind it out of nowhere.
      {
        path: 'xinutec-infra/plan/types.dhall',
        was: ['xinutec-infra/plan/schema.dhall'],
        reads: 0,
        edits: 7,
        shell_reads: 0,
        shell_edits: 0,
        added: 300,
        deleted: 12,
        commits: 4,
      },
      // Changed from the shell as well as from `Edit`, which is what the
      // provenance line beside the totals exists to show.
      {
        path: 'xinutec-infra/plan/deploy.dhall',
        reads: 1,
        edits: 2,
        shell_reads: 1,
        shell_edits: 1,
        added: 0,
        deleted: 0,
        commits: 0,
      },
      // On another machine entirely — the row must say so.
      {
        path: '/etc/nixos/flake.nix',
        host: 'odin',
        reads: 3,
        edits: 1,
        shell_reads: 3,
        shell_edits: 1,
        added: 0,
        deleted: 0,
        commits: 0,
      },
    ],
  },
  {
    name: 'home',
    edits: 25,
    reads: 2,
    added: 0,
    deleted: 0,
    file_commits: 0,
    hosts: [],
    files: [
      {
        path: 'k/dhall/home.dhall',
        reads: 2,
        edits: 25,
        shell_reads: 0,
        shell_edits: 0,
        added: 0,
        deleted: 0,
        commits: 0,
      },
    ],
  },
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
    expect(paths.map((p) => p?.replace(/\s+/g, ''))).toEqual([
      // The first carries its former name, which is part of what the file IS.
      'xinutec-infra/plan/types.dhallwasxinutec-infra/plan/schema.dhall',
      'xinutec-infra/plan/deploy.dhall',
      'odin:/etc/nixos/flake.nix',
    ]);
  });

  it('names a file that used to be called something else', async () => {
    // Git is the only evidence that two names are one file. Without saying so,
    // a file created last week appears to carry a year of history from nowhere.
    const el = await search(WORKERS);
    el.querySelector<HTMLButtonElement>('.workers .worker')!.click();
    await fixture.whenStable();

    const was = [...el.querySelectorAll('.workers .file-list .was')].map((n) =>
      n.textContent?.trim(),
    );
    expect(was).toEqual(['was xinutec-infra/plan/schema.dhall']);
  });

  it('says which of a file’s changes came from the shell, and only where some did', async () => {
    // The totals include shell work, so a file can show changes no `Edit` ever
    // made. Unlabelled that reads as a counting bug rather than as somebody
    // working through `sed` — and the label must not appear where it would be
    // two zeroes of noise.
    const el = await search(WORKERS);
    el.querySelector<HTMLButtonElement>('.workers .worker')!.click();
    await fixture.whenStable();

    const shell = [...el.querySelectorAll('.workers .file-list .via-shell')].map((n) =>
      n.textContent?.trim(),
    );
    expect(shell).toEqual(['1×w · 1×r in the shell']);
  });

  it('reports committed lines as their own measure, only where there are any', async () => {
    // Lines are size; the counts beside them are frequency. They are never
    // added together — that would count the same work twice — and an agent
    // whose work was never committed shows nothing rather than a zero, which
    // would read as "committed nothing" instead of "git cannot say".
    const el = await search(WORKERS);
    const rows = [...el.querySelectorAll('.workers .worker .lines')].map((n) => n.textContent);
    expect(rows).toEqual(['+2126/\u2212433']);

    el.querySelector<HTMLButtonElement>('.workers .worker')!.click();
    await fixture.whenStable();
    const files = [...el.querySelectorAll('.workers .file-list .lines')].map((n) => n.textContent);
    expect(files).toEqual(['+300/\u221212 in 4 commits']);
  });

  it('renders a hit’s markdown rather than showing its punctuation', async () => {
    // Snippets and descriptions arrive as inline HTML, because raw they read as
    // punctuation: backticks around a path, asterisks around a bold run. The
    // binding must place elements, not escape them into text.
    fixture.componentRef.instance.query.set('dhall');
    fixture.componentRef.instance.submit();
    await fixture.whenStable();
    http
      .expectOne((r) => r.url === '/api/search')
      .flush({
        hits: [
          {
            name: 'project_kubes_dhall_model',
            description: '<code>code/kubes/dhall/</code> models the fleet',
            mtype: 'project',
            modified: null,
            created: null,
            snippet: '…<strong>How to apply:</strong> for new automation',
            score: 1,
          },
        ],
        relaxed: false,
      });
    http.expectOne((r) => r.url === '/api/work').flush([]);
    await fixture.whenStable();

    const el = fixture.nativeElement as HTMLElement;
    expect(el.querySelector('.hit-list .desc code')?.textContent).toBe('code/kubes/dhall/');
    expect(el.querySelector('.hit-list .snippet strong')?.textContent).toBe('How to apply:');
    // ...and no leftover markers anywhere in the rendered hit.
    expect(el.querySelector('.hit-list')?.textContent).not.toContain('`');
    expect(el.querySelector('.hit-list')?.textContent).not.toContain('**');
  });

  it('says which machine a file is on, and only when it is not this one', async () => {
    // "This path is not on this computer" is the single most misreadable thing
    // in the list: /etc/nixos/flake.nix looks local and is not.
    const el = await search(WORKERS);
    expect(el.querySelector('.workers .hosts')?.textContent?.trim()).toBe('on odin');

    el.querySelector<HTMLButtonElement>('.workers .worker')!.click();
    await fixture.whenStable();
    const hosts = [...el.querySelectorAll('.workers .file-list .host')].map((n) => n.textContent);
    expect(hosts).toEqual(['odin:']);
  });

  it('shows no panel to a share-link recipient, and no error either', async () => {
    // 403 is the intended answer for a share token, not a failure: the roster is
    // owner-only. It must read as "not yours to see", never as a broken search.
    const el = await search('forbidden');
    expect(el.querySelector('.workers')).toBeNull();
    expect(el.textContent).not.toContain('Who works on this');
  });
});
