import { beforeEach, describe, expect, it } from 'vitest';
import { provideZonelessChangeDetection } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { provideRouter } from '@angular/router';

import { AgentsView } from './agents-view';
import { Agent, AgentsResult } from './models';
import { routes } from './app.routes';

/**
 * One agent that does most of its work where no tool call can see it — a
 * `sed -i` here, a `python3 -` heredoc there, and a machine that is not this
 * one. Before the four dimensions were unioned, this row read as a session that
 * had barely touched `health` at all.
 */
const AGENT: Agent = {
  name: 'health',
  transcripts: 2,
  delegated: 1,
  sessions: ['a'],
  paths: { 'health/src/geo/osm.ts': { reads: 4, edits: 1 } },
  shell_paths: {
    'health/src/geo/velocity.ts': { reads: 6, edits: 9, maybe_reads: 5, maybe_edits: 3 },
    // A glob is recorded as it was written. Honest for a file list, useless as
    // a project name — it must not become a project called `*`.
    '*/*/android': { reads: 1, edits: 0 },
  },
  remote_paths: {
    'odin:/etc/nixos/configuration.nix': { reads: 3, edits: 2 },
    'odin:/etc/nixos/flake.nix': { reads: 1, edits: 0 },
    'isis:/srv/x.yaml': { reads: 1, edits: 0 },
  },
  commit_lines: { 'health/src/geo/velocity.ts': { added: 300, deleted: 12, commits: 4 } },
  commits: 4,
  reads: { health: 4 },
  writes: { health: 1 },
  memories: {},
  recent_reads: { health: 1 },
  recent_writes: { health: 1 },
  first: '2026-07-01T00:00:00Z',
  last: '2026-08-02T00:00:00Z',
};

const RESULT: AgentsResult = {
  generated: '2026-08-02T12:00:00Z',
  renames: {},
  commits: 10,
  unattributed: 1,
  agents: [AGENT],
};

describe('AgentsView — every dimension of the evidence', () => {
  let fixture: ComponentFixture<AgentsView>;
  let http: HttpTestingController;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [AgentsView],
      providers: [
        provideZonelessChangeDetection(),
        provideHttpClient(),
        provideHttpClientTesting(),
        provideRouter(routes),
      ],
    }).compileComponents();
    fixture = TestBed.createComponent(AgentsView);
    http = TestBed.inject(HttpTestingController);
    fixture.detectChanges();
    http.expectOne('/api/agents').flush(RESULT);
    await fixture.whenStable();
    fixture.detectChanges();
  });

  it('counts shell work into the totals and says how much of it there was', () => {
    const row = fixture.componentInstance.rows()[0];
    // 4 tool reads + 6 shell reads; 1 tool edit + 9 shell edits. The old page
    // said 4 and 1, which is the undercount the shell reader exists to fix.
    expect(row.reads).toBe(10);
    expect(row.writes).toBe(10);
    expect(row.shellReads).toBe(6);
    expect(row.shellWrites).toBe(9);
  });

  it('keeps uses that may never have happened out of every count they are not', () => {
    const row = fixture.componentInstance.rows()[0];
    // ⚠ The property the whole separation exists for. The fixture's 5 maybe
    // reads and 3 maybe edits are carried, and are in NEITHER the totals nor the
    // shell breakdown — those stay at the 10/10 and 6/9 asserted above. Summing
    // them would spend the distinction the miner went to the trouble of keeping:
    // a certain read happened, a maybe read is a command that may never have run.
    expect(row.maybeReads).toBe(5);
    expect(row.maybeWrites).toBe(3);
    expect(row.reads).toBe(10);
    expect(row.writes).toBe(10);
    expect(row.shellReads).toBe(6);
    expect(row.shellWrites).toBe(9);
    expect(row.places[0]).toMatchObject({ maybeReads: 5, maybeWrites: 3, reads: 10, writes: 10 });
  });

  it('draws the uncertainty inside the shell phrase and nowhere else', () => {
    // Measured over the artefact: `paths`, `remote_paths` and `memories` carry 0
    // maybes without exception, because a tool call is atomic and its result says
    // which way it went. So the uncertainty belongs to the shell figure it
    // qualifies — the machines line and the memory list have none to draw, and an
    // always-zero figure beside them would imply one that does not exist.
    const el = fixture.nativeElement as HTMLElement;
    expect(el.querySelector('.totals .maybe')?.textContent).toContain('that may not have run');
    expect(el.querySelector('.places .maybe')?.textContent).toContain('3w unproven');
    expect(el.querySelectorAll('.machines .maybe')).toHaveLength(0);
    expect(el.querySelectorAll('.knows .maybe')).toHaveLength(0);
  });

  it('carries committed lines, which are size where the counts are frequency', () => {
    const row = fixture.componentInstance.rows()[0];
    expect(row.added).toBe(300);
    expect(row.deleted).toBe(12);
    expect(row.commits).toBe(4);
    expect(row.places[0]).toMatchObject({ project: 'health', added: 300, deleted: 12 });
  });

  it('keeps other machines out of the projects and names them separately', () => {
    const row = fixture.componentInstance.rows()[0];
    expect(row.places.map((p) => p.project)).toEqual(['health']);
    expect(row.machines).toEqual([
      { host: 'odin', reads: 4, writes: 2 },
      { host: 'isis', reads: 1, writes: 0 },
    ]);
  });

  it('renders the provenance rather than only the total', () => {
    const text = (fixture.nativeElement as HTMLElement).textContent ?? '';
    expect(text).toContain('through the shell');
    expect(text).toContain('+300/−12 in 4 commits');
    expect(text).toContain('odin');
  });
});
