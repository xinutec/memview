import { HttpClient } from '@angular/common/http';
import { Injectable, inject } from '@angular/core';
import { Observable } from 'rxjs';

import { Conversation, KINDS, Overview, SessionEvent, Summary } from './models';

/** Thin client over the console runner. Same origin in production (the runner
 *  serves this bundle); via the dev proxy under `ng serve`. */
@Injectable({ providedIn: 'root' })
export class ConsoleApi {
  private http = inject(HttpClient);

  state(): Observable<Overview> {
    return this.http.get<Overview>('/api/state');
  }

  /** The page of conversation before the one the caller holds. */
  /**
   * The page of transcript before the one the reader holds.
   *
   * `before` is the cursor that page arrived with — never a count of anything
   * this client has. It was a count, and the two ends counted different things:
   * the runner counts events, the page holds folded entries, and several text
   * deltas are one paragraph. Both were numbers, so the mismatch was invisible
   * and the feature returned the reader's own screen back to them, forever. The
   * cursor is opaque here on purpose — there is nothing to compute it from, so
   * it cannot be computed wrongly.
   */
  earlier(id: string, before: number): Observable<{ events: SessionEvent[]; from: number }> {
    return this.http.get<{ events: SessionEvent[]; from: number }>(
      `/api/sessions/${encodeURIComponent(id)}/earlier`,
      { params: { before } },
    );
  }

  past(): Observable<Conversation[]> {
    return this.http.get<Conversation[]>('/api/past');
  }

  start(dir: string, prompt: string, resume?: string): Observable<Summary> {
    return this.http.post<Summary>('/api/sessions', { dir, prompt, resume });
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
    // ⚠ Not `onopen`. A reconnect used to mean "throw everything away", because
    // the server had no way to send only what was missed — so a phone going
    // through a tunnel discarded the history somebody had just scrolled back to
    // load. The events are numbered now and the browser quotes the last one back
    // on its own, so a reconnect is ordinarily seamless and this fires only when
    // the runner says it genuinely cannot resume: a console restarted, or a
    // session busy enough to have dropped that far out of its scrollback.
    source.addEventListener('reset', () => onReset());
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
