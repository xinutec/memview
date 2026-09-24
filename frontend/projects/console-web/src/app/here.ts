import { Injectable, signal } from '@angular/core';

import { Gist, Summary, TaskCount } from './models';

/** A place to go back to, as a router link. */
export interface Up {
  readonly path: string;
  readonly query?: Readonly<Record<string, string>>;
}

export const LIST: Up = { path: '/' };

/**
 * Which conversation the reader is looking at, for the parts of the shell that
 * sit above the router and cannot ask it.
 */
@Injectable({ providedIn: 'root' })
export class Here {
  /** The open conversation, or nothing when no session is on screen. */
  readonly open = signal<Summary | undefined>(undefined);

  /**
   * The session the ROUTE names. [open] waits on `/api/state`, so which screen
   * you are on is a fact about the URL.
   */
  readonly at = signal<string | undefined>(undefined);

  /** A screen that is neither the list nor a session, by the name its bar should carry. */
  readonly page = signal<string | undefined>(undefined);

  /** Where that screen's back arrow goes: the list, unless it was reached from a session. */
  readonly up = signal<Up>(LIST);

  /**
   * What this conversation is about, when a sentence has been written for it.
   * Beside the summary, not on it: gists cover the transcripts on disk too.
   */
  readonly gist = signal<Gist | undefined>(undefined);

  /** How much is left of this conversation's list. Keyed by conversation, as [gist] is. */
  readonly tasks = signal<TaskCount | undefined>(undefined);
}
