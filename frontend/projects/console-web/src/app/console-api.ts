import { HttpClient } from '@angular/common/http';
import { Injectable, inject } from '@angular/core';
import { Observable } from 'rxjs';

import {
  CALLS,
  type Conversation,
  type CorpusRead,
  type Decision,
  type Described,
  type EditRecord,
  KINDS,
  type Landmark,
  type Message,
  type Mode,
  type Overview,
  type Page,
  type Parsed,
  type Renaming,
  type Run,
  type Shown,
  type Start,
  type Stretched,
  type Summary,
  type Task,
  type Timed,
} from './models';
import { fetchedAt } from './picture';
import { type Answers, type Notes } from './questions';

/** What the event stream yields: a message, or a change in the connection. */
export type Streamed =
  | { kind: 'event'; event: Timed; seq: number }
  | { kind: 'reset' }
  | { kind: 'caught-up' }
  | { kind: 'offline' };

@Injectable({ providedIn: 'root' })
export class ConsoleApi {
  private readonly http = inject(HttpClient);

  state(): Observable<Overview> {
    return this.http.get<Overview>('/api/state');
  }

  reading(): Observable<CorpusRead> {
    return this.http.get<CorpusRead>('/api/reading');
  }

  /** The page of events ending at `before`, a cursor from a previous page. */
  earlier(id: string, before: number): Observable<Page> {
    return this.http.get<Page>(`${session(id)}/earlier`, { params: { before } });
  }

  landmarks(id: string): Observable<Landmark[]> {
    return this.http.get<Landmark[]>(`${session(id)}/landmarks`);
  }

  past(): Observable<Conversation[]> {
    return this.http.get<Conversation[]>('/api/past');
  }

  tasks(id: string): Observable<Task[]> {
    return this.http.get<Task[]>(`${session(id)}/tasks`);
  }

  task(id: string, task: string): Observable<Described> {
    return this.http.get<Described>(`${session(id)}/tasks/${encodeURIComponent(task)}`);
  }

  /** What each `Bash` call of a conversation was predicted to change, and which diverged. */
  edits(id: string): Observable<EditRecord> {
    return this.http.get<EditRecord>(`${session(id)}/edits`);
  }

  /** A workflow run the session launched: its agents by phase. */
  workflow(id: string, run: string): Observable<Run> {
    return this.http.get<Run>(`${session(id)}/workflows/${encodeURIComponent(run)}`);
  }

  /**
   * A stretch of one workflow agent's transcript: the newest page, the page
   * before `before`, or what came after `after`.
   */
  agent(
    id: string,
    run: string,
    agent: string,
    at: { before: number } | { after: number } | Record<string, never> = {},
  ): Observable<Stretched> {
    return this.http.get<Stretched>(
      `${session(id)}/workflows/${encodeURIComponent(run)}/agents/${encodeURIComponent(agent)}`,
      { params: at },
    );
  }

  parse(id: string, command: string, ok?: boolean): Observable<Parsed> {
    return this.http.post<Parsed>(`${session(id)}/parse`, { command, ok });
  }

  start(dir: string, prompt: string, resume?: string): Observable<Summary> {
    return this.http.post<Summary>('/api/sessions', { dir, prompt, resume } satisfies Start);
  }

  send(id: string, text: string): Observable<Summary> {
    return this.http.post<Summary>(`${session(id)}/input`, { text } satisfies Message);
  }

  show(id: string, data: string, mediaType: string, text: string): Observable<Summary> {
    const body: Shown = { data, media_type: mediaType, text };
    return this.http.post<Summary>(`${session(id)}/image`, body);
  }

  pictureAt(id: string, name: string): string {
    return `${session(id)}/images/${encodeURIComponent(name)}`;
  }

  /** An image from anywhere on the web, fetched by the runner rather than the phone. */
  elsewhere(url: string): Observable<Blob> {
    return this.http.get(fetchedAt(url), { responseType: 'blob' });
  }

  decide(
    at: string,
    id: string,
    allow: boolean,
    why?: string,
    answers?: Answers,
    response?: string,
    notes?: Notes,
  ): Observable<Summary> {
    const body: Decision = {
      id,
      allow,
      why,
      answers,
      response,
      annotations:
        notes && Object.fromEntries(Object.entries(notes).map(([q, n]) => [q, { notes: n }])),
    };
    return this.http.post<Summary>(`${session(at)}/decide`, body);
  }

  setMode(id: string, mode: string): Observable<Summary> {
    return this.http.post<Summary>(`${session(id)}/mode`, { mode } satisfies Mode);
  }

  rename(id: string, title: string): Observable<Summary> {
    return this.http.post<Summary>(`${session(id)}/rename`, { title } satisfies Renaming);
  }

  unhold(id: string, text: string): Observable<Summary> {
    return this.http.post<Summary>(`${session(id)}/unhold`, { text } satisfies Message);
  }

  stop(id: string): Observable<Summary> {
    return this.http.post<Summary>(`${session(id)}/stop`, {});
  }

  revive(id: string): Observable<Summary> {
    return this.http.post<Summary>(`${session(id)}/revive`, {});
  }

  forget(id: string): Observable<unknown> {
    return this.http.delete(session(id));
  }

  /** The session's events from `after`, then live, as server-sent events. */
  follow(id: string, after: number): Observable<Streamed> {
    return new Observable<Streamed>((to) => {
      const url = `${session(id)}/events`;
      const source = new EventSource(after > 0 ? `${url}?after=${after}` : url);
      source.addEventListener('reset', () => to.next({ kind: 'reset' }));
      source.addEventListener('caught-up', () => to.next({ kind: 'caught-up' }));
      source.onerror = () => to.next({ kind: 'offline' });
      source.onmessage = (message: MessageEvent<unknown>) => {
        const event = parse(message.data);
        if (event) {
          to.next({ kind: 'event', event, seq: Number.parseInt(message.lastEventId, 10) || 0 });
        }
      };
      return () => source.close();
    });
  }
}

function session(id: string): string {
  return `/api/sessions/${encodeURIComponent(id)}`;
}

/**
 * Narrow a message from the wire, or drop it.
 *
 * The one place the shape is checked: `kind` against the list of what the
 * runner sends, so a variant added there without a generated type here is
 * refused at the boundary rather than drawn wrongly.
 */
/** Whether a `does` names a kind of call this client draws. */
function isCall(does: unknown): boolean {
  if (typeof does !== 'object' || does === null || !('kind' in does)) return false;
  return CALLS.some((known) => known === does.kind);
}

function parse(data: unknown): Timed | undefined {
  if (typeof data !== 'string') return undefined;
  let value: unknown;
  try {
    value = JSON.parse(data);
  } catch {
    return undefined;
  }
  if (typeof value !== 'object' || value === null || !('kind' in value)) return undefined;
  const kind = KINDS.find((known) => known === value.kind);
  if (!kind) return undefined;
  if ('does' in value && !isCall(value.does)) return undefined;
  // The trust boundary: the kind is checked above, the fields are the runner's word.
  // eslint-disable-next-line @typescript-eslint/no-unsafe-type-assertion
  return { ...value, kind } as Timed;
}
