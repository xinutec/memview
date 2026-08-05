import { HttpClient } from '@angular/common/http';
import { Injectable, inject } from '@angular/core';
import { Observable } from 'rxjs';

import { Conversation, KINDS, Overview, SessionEvent, Summary, Task } from './models';
import { Answers, Notes } from './questions';

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

  /** A session's task list, subjects only — see [[ConsoleApi.task]]. */
  tasks(id: string): Observable<Task[]> {
    return this.http.get<Task[]>(`/api/sessions/${encodeURIComponent(id)}/tasks`);
  }

  /**
   * What one task says, fetched when it is opened.
   *
   * ⚠ **Not sent with the list, and that is not a micro-optimisation.** These
   * descriptions are written-up results running to kilobytes each — one live
   * session's 355 tasks are 1.5 MB of them, which is not a payload for drawing
   * forty subjects on a phone.
   */
  task(id: string, task: string): Observable<{ description: string }> {
    return this.http.get<{ description: string }>(
      `/api/sessions/${encodeURIComponent(id)}/tasks/${encodeURIComponent(task)}`,
    );
  }

  start(dir: string, prompt: string, resume?: string): Observable<Summary> {
    return this.http.post<Summary>('/api/sessions', { dir, prompt, resume });
  }

  send(id: string, text: string): Observable<Summary> {
    return this.http.post<Summary>(`/api/sessions/${encodeURIComponent(id)}/input`, { text });
  }

  /** Answer a question the session is blocked on.
   *
   *  `answers` are the choices made about an `AskUserQuestion`, and the runner
   *  refuses them for anything else — approving a tool call is not a licence to
   *  rewrite it. See `questions.ts` for why an answer travels this way at all.
   */
  decide(
    session: string,
    id: string,
    allow: boolean,
    why?: string,
    answers?: Answers,
    response?: string,
    notes?: Notes,
  ): Observable<Summary> {
    return this.http.post<Summary>(`/api/sessions/${encodeURIComponent(session)}/decide`, {
      id,
      allow,
      why,
      answers,
      response,
      // The wire shape is the CLI's, which nests each note in an object of its
      // own — `preview` is the other thing that can live there, and is the
      // terminal picker's business rather than ours.
      annotations:
        notes && Object.fromEntries(Object.entries(notes).map(([q, n]) => [q, { notes: n }])),
    });
  }

  /** Change what a session may do without asking. See `modes.ts`. */
  setMode(id: string, mode: string): Observable<Summary> {
    return this.http.post<Summary>(`/api/sessions/${encodeURIComponent(id)}/mode`, { mode });
  }

  stop(id: string): Observable<Summary> {
    return this.http.post<Summary>(`/api/sessions/${encodeURIComponent(id)}/stop`, {});
  }

  forget(id: string): Observable<unknown> {
    return this.http.delete(`/api/sessions/${encodeURIComponent(id)}`);
  }

  /** Follow a session, from where the caller left off to now and onward.
   *
   *  EventSource rather than a polling GET: the answer arrives while it is being
   *  written, and it reconnects on its own when the phone changes network —
   *  which is the normal case, not the exception.
   *
   *  `after` is the last sequence number the caller holds, and it is what makes
   *  closing the stream survivable. The browser quotes the last id back by itself
   *  on a reconnect, but only for the same `EventSource` — a new one opened by a
   *  page returning to a session it had left knows nothing, so it has to say. The
   *  answer is the same either way: the events since, or a `reset` when the
   *  runner cannot bridge the gap.
   *
   *  `onEvent` is handed the number alongside the event, because the caller is
   *  the only one that knows when it has finished with it. Zero for the events
   *  the runner deliberately leaves unnumbered — see the drop notice in `api.rs`.
   *
   *  Returns the function that closes it. */
  follow(
    id: string,
    after: number,
    onEvent: (event: SessionEvent, seq: number) => void,
    onReset: () => void,
  ): () => void {
    const url = `/api/sessions/${encodeURIComponent(id)}/events`;
    const source = new EventSource(after > 0 ? `${url}?after=${after}` : url);
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
      // `lastEventId` is '' on the unnumbered ones, and `Number('')` is 0 — which
      // is why the number is taken through `parseInt`, whose answer for a
      // non-number is NaN and is rejected here rather than becoming a sequence
      // the caller would then claim to hold.
      if (event) onEvent(event, Number.parseInt(message.lastEventId, 10) || 0);
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
  if ('input' in value && (typeof value.input !== 'object' || value.input === null))
    return undefined;
  return { ...value, kind };
}
