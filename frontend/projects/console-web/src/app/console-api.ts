import { HttpClient } from '@angular/common/http';
import { Injectable, inject } from '@angular/core';
import { Observable } from 'rxjs';

import { KINDS, Overview, SessionEvent, Summary } from './models';

/** Thin client over the console runner. Same origin in production (the runner
 *  serves this bundle); via the dev proxy under `ng serve`. */
@Injectable({ providedIn: 'root' })
export class ConsoleApi {
  private http = inject(HttpClient);

  state(): Observable<Overview> {
    return this.http.get<Overview>('/api/state');
  }

  start(dir: string, prompt: string): Observable<Summary> {
    return this.http.post<Summary>('/api/sessions', { dir, prompt });
  }

  send(id: string, text: string): Observable<Summary> {
    return this.http.post<Summary>(`/api/sessions/${encodeURIComponent(id)}/input`, { text });
  }

  /** Answer a question the session is blocked on. */
  decide(session: string, id: string, allow: boolean, why?: string): Observable<Summary> {
    return this.http.post<Summary>(`/api/sessions/${encodeURIComponent(session)}/decide`, {
      id,
      allow,
      why,
    });
  }

  stop(id: string): Observable<Summary> {
    return this.http.post<Summary>(`/api/sessions/${encodeURIComponent(id)}/stop`, {});
  }

  forget(id: string): Observable<unknown> {
    return this.http.delete(`/api/sessions/${encodeURIComponent(id)}`);
  }

  /** Follow a session, from the beginning of its transcript to now and onward.
   *
   *  EventSource rather than a polling GET: the answer arrives while it is being
   *  written, and it reconnects on its own when the phone changes network —
   *  which is the normal case, not the exception. The server replays the whole
   *  transcript to every new connection, so a reconnect is not a gap; it is why
   *  the caller is handed a fresh list each time rather than appending to what
   *  it had.
   *
   *  Returns the function that closes it. */
  follow(id: string, onEvent: (event: SessionEvent) => void, onReset: () => void): () => void {
    const source = new EventSource(`/api/sessions/${encodeURIComponent(id)}/events`);
    let opened = false;
    source.onopen = () => {
      // The first open is the initial load; a later one is a reconnect, after
      // which the server starts again from the top of the transcript.
      if (opened) onReset();
      opened = true;
    };
    source.onmessage = (message: MessageEvent<unknown>) => {
      const event = parse(message.data);
      // A line that is not an event this version knows is dropped rather than
      // rendered: the runner reports its own failures as `trouble` events, and
      // one unreadable line must not end the stream.
      if (event) onEvent(event);
    };
    return () => source.close();
  }
}

/** Narrow a message from the wire, or reject it.
 *
 *  The boundary where an unknown becomes a typed event, and the only place the
 *  shape is checked — `kind` against the list the runner can actually send, so
 *  a value that never reaches a template cannot arrive here unnoticed. */
function parse(data: unknown): SessionEvent | undefined {
  if (typeof data !== 'string') return undefined;
  let value: unknown;
  try {
    value = JSON.parse(data);
  } catch {
    return undefined;
  }
  if (typeof value !== 'object' || value === null) return undefined;
  if (!('kind' in value)) return undefined;
  // Matched against the list rather than asserted: `find` yields the literal
  // type, which is what makes the returned object an event without a cast.
  const kind = KINDS.find((known) => known === value.kind);
  if (!kind) return undefined;
  if ('input' in value && (typeof value.input !== 'object' || value.input === null)) return undefined;
  return { ...value, kind };
}
