import { Injectable, signal } from '@angular/core';

import { Gist, Summary, TaskCount } from './models';

/**
 * Which conversation the reader is looking at, for the parts of the shell that
 * sit above the router and cannot ask it.
 *
 * The whole summary rather than the fields the toolbar reads: the toolbar is
 * also the way into the details sheet, which shows nearly all of it.
 */
@Injectable({ providedIn: 'root' })
export class Here {
  /** The open conversation, or nothing when no session is on screen. */
  readonly open = signal<Summary | undefined>(undefined);

  /**
   * The session the ROUTE names.
   *
   * ⚠ **[open] is not the answer to "am I in a session".** It waits on
   * `/api/state`, so a cold launch straight into a conversation draws the list's
   * toolbar for one round trip. Which screen you are on is a fact about the URL.
   */
  readonly at = signal<string | undefined>(undefined);

  /** A screen that is neither the list nor a session, by the name its bar should
   *  carry. From the route, like [at] and for the same reason. */
  readonly page = signal<string | undefined>(undefined);

  /**
   * What this conversation is about, when a sentence has been written for it.
   *
   * ⚠ **Beside the summary, not on it.** These arrive keyed by conversation
   * because they cover the transcripts on disk, which have no summary.
   */
  readonly gist = signal<Gist | undefined>(undefined);

  /** How much is left of this conversation's list. Keyed by conversation, as
   *  [gist] is and for the same reason. */
  readonly tasks = signal<TaskCount | undefined>(undefined);
}
