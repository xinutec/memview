import { HttpClient, HttpParams } from '@angular/common/http';
import { Injectable, inject } from '@angular/core';
import { Observable } from 'rxjs';

import {
  GraphData,
  IndexPage,
  Me,
  MemoryMeta,
  MemoryPage,
  AgentsResult,
  CorpusRead,
  Evidence,
  SearchResult,
  ShareInfo,
  Timeline,
  WorkMatch,
} from './models';

/** Thin client over the memview backend. Same-origin in prod; via the dev
 *  proxy (proxy.conf.json) in `ng serve`. Session cookie rides along. */
@Injectable({ providedIn: 'root' })
export class MemviewApi {
  private http = inject(HttpClient);

  me(): Observable<Me> {
    return this.http.get<Me>('/api/me');
  }
  logout(): Observable<unknown> {
    return this.http.post('/logout', {});
  }

  index(): Observable<IndexPage> {
    return this.http.get<IndexPage>('/api/index');
  }

  memories(): Observable<MemoryMeta[]> {
    return this.http.get<MemoryMeta[]>('/api/memories');
  }

  memory(name: string): Observable<MemoryPage> {
    return this.http.get<MemoryPage>(`/api/memory/${encodeURIComponent(name)}`);
  }

  graph(): Observable<GraphData> {
    return this.http.get<GraphData>('/api/graph');
  }

  search(q: string): Observable<SearchResult> {
    return this.http.get<SearchResult>('/api/search', { params: { q } });
  }

  /**
   * Who has been changing the files a query names. Owner-only; a share token
   * gets 403, which the search view treats as "no panel" rather than an error.
   */
  work(q: string): Observable<WorkMatch[]> {
    return this.http.get<WorkMatch[]>('/api/work', { params: { q } });
  }

  shareGet(): Observable<ShareInfo> {
    return this.http.get<ShareInfo>('/api/share');
  }
  shareRotate(): Observable<ShareInfo> {
    return this.http.post<ShareInfo>('/api/share', {});
  }
  /** Which named session works where. Owner-only; a share token gets 403. */
  agents(): Observable<AgentsResult> {
    return this.http.get<AgentsResult>('/api/agents');
  }

  /**
   * What the sessions did, newest first. Owner-only.
   *
   * Every filter is optional and they compose. A filter naming something the
   * corpus has never seen matches NOTHING rather than everything — the server
   * decides that, and the page must not "helpfully" drop an unmatched filter,
   * or "no such agent" would render as the whole history.
   */
  doing(filter: {
    agent?: string;
    project?: string;
    kind?: string;
    before?: number;
  }): Observable<Timeline> {
    let params = new HttpParams();
    for (const [key, value] of Object.entries(filter)) {
      if (value !== undefined && value !== '') params = params.set(key, String(value));
    }
    return this.http.get<Timeline>('/api/doing', { params });
  }

  /**
   * What one turn did, keyed by the `(agent, at)` its timeline row carries.
   *
   * A filter, not a join: the row already holds both halves of the key, so
   * opening it costs one request and no lookup table.
   */
  effects(agent: string, at: number): Observable<Evidence> {
    return this.http.get<Evidence>('/api/effects', {
      params: new HttpParams().set('agent', agent).set('at', String(at)),
    });
  }

  /**
   * What the reader makes of every shell command the fleet has run.
   *
   * One request, ~7 kB, and no parameters: the artefact is already the summary.
   * A 404 means nothing has been mined yet, which the view says out loud rather
   * than drawing as zeroes.
   */
  reading(): Observable<CorpusRead> {
    return this.http.get<CorpusRead>('/api/reading');
  }

  shareRevoke(): Observable<ShareInfo> {
    return this.http.delete<ShareInfo>('/api/share');
  }
}
