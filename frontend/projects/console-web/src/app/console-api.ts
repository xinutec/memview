import { HttpClient } from '@angular/common/http';
import { Injectable, inject } from '@angular/core';
import { Observable } from 'rxjs';

import { Conversation, KINDS, Overview, Parsed, SessionEvent, Summary, Task } from './models';
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

  /**
   * One `Bash` command, read by the same library the index is built from.
   *
   * ⚠ **The working directory is deliberately not sent.** The runner takes it
   * from the session, because a relative operand resolves against it and a
   * client free to choose it could make the answer name any file it liked —
   * where the whole worth of this view is that it says what the miner would say.
   *
   * `ok` is the call's own verdict, `undefined` while it is still running. That
   * is a third state and not a synonym for failure: half of what this view shows
   * is which uses the outcome makes certain.
   */
  parse(id: string, command: string, ok?: boolean): Observable<Parsed> {
    return this.http.post<Parsed>(`/api/sessions/${encodeURIComponent(id)}/parse`, { command, ok });
  }

  start(dir: string, prompt: string, resume?: string): Observable<Summary> {
    return this.http.post<Summary>('/api/sessions', { dir, prompt, resume });
  }

  send(id: string, text: string): Observable<Summary> {
    return this.http.post<Summary>(`/api/sessions/${encodeURIComponent(id)}/input`, { text });
  }

  /**
   * Show a session a picture, with whatever is being said about it.
   *
   * ⚠ **Its own route, not a field on `send`.** This one is a megabyte where that
   * one is a sentence, the runner writes a file for it, and it fails for reasons
   * — too large, not an image — that have no meaning for text. `data` is bare
   * base64 rather than a data URL: the runner hands it straight to the CLI, which
   * wants it the way the API defines it.
   */
  show(id: string, data: string, mediaType: string, text: string): Observable<Summary> {
    return this.http.post<Summary>(`/api/sessions/${encodeURIComponent(id)}/image`, {
      data,
      media_type: mediaType,
      text,
    });
  }

  /** Where a picture that was sent to a session can be fetched from.
   *
   *  A URL rather than a request: it is handed to an `<img>`, which does the
   *  fetching, caching and decoding that this service would otherwise be
   *  reimplementing. The runner serves these `immutable` — a kept picture is
   *  written once under the second it arrived in and never rewritten — so
   *  scrolling back through a conversation costs nothing after the first look.
   */
  pictureAt(id: string, name: string): string {
    return `/api/sessions/${encodeURIComponent(id)}/images/${encodeURIComponent(name)}`;
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

  /** Rename a conversation, including one that is working.
   *
   *  ⚠ **Not `/rename`.** A slash command sent to a busy session is parked and
   *  handed to the model as words — measured: the agent replied "nothing for me
   *  to do" and the name never changed. This goes over the control channel,
   *  which is answered whatever the turn is doing. See `protocol::rename`.
   *
   *  ⚠ **The answer still carries the OLD name.** The CLI writes the new one to
   *  the transcript and the runner reads names from there, so it arrives on the
   *  next poll — a second or so. Anything that redraws from this response alone
   *  will look like it did nothing. */
  rename(id: string, title: string): Observable<Summary> {
    return this.http.post<Summary>(`/api/sessions/${encodeURIComponent(id)}/rename`, { title });
  }

  stop(id: string): Observable<Summary> {
    return this.http.post<Summary>(`/api/sessions/${encodeURIComponent(id)}/stop`, {});
  }

  /** Stop a session that has stopped listening and start it again on the same
   *  conversation, handing back the messages it never read.
   *
   *  ⚠ **Tens of seconds, not a moment.** The server waits for the old process
   *  to leave the process table before resuming, because two processes on one
   *  transcript both append and neither sees the other. Whatever calls this has
   *  to stay disabled and say what it is doing until it answers. */
  revive(id: string): Observable<Summary> {
    return this.http.post<Summary>(`/api/sessions/${encodeURIComponent(id)}/revive`, {});
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
    onCaughtUp: () => void,
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
    // ⚠ **Where the replay ends and the present begins.** Everything before it
    // is the transcript being caught up on, and a replayed `turn` is
    // indistinguishable from one that just ended — which is how the page came to
    // report `idle` over twelve minutes of work. Named rather than a domain
    // event because it is a fact about this connection, and per connection
    // rather than in the log because the log can be trimmed out from under a
    // client that joins late.
    source.addEventListener('caught-up', () => onCaughtUp());
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
