import { HttpClient } from '@angular/common/http';
import { Injectable, inject } from '@angular/core';
import { Observable } from 'rxjs';

import { IndexPage, Me, MemoryMeta, MemoryPage, SearchHit, ShareInfo } from './models';

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

  search(q: string): Observable<{ hits: SearchHit[] }> {
    return this.http.get<{ hits: SearchHit[] }>('/api/search', { params: { q } });
  }

  shareGet(): Observable<ShareInfo> {
    return this.http.get<ShareInfo>('/api/share');
  }
  shareRotate(): Observable<ShareInfo> {
    return this.http.post<ShareInfo>('/api/share', {});
  }
  shareRevoke(): Observable<ShareInfo> {
    return this.http.delete<ShareInfo>('/api/share');
  }
}
