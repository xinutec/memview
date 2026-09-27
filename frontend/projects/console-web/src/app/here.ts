import { Injectable, signal } from '@angular/core';

import { Gist, Summary, TaskCount } from './models';

/**
 * Which conversation the reader is looking at, for the parts of the app that sit
 * above the router and cannot ask it. Where up goes and what the bar is titled
 * are the scaffold's (`@xinutec/ui-scaffold`).
 */
@Injectable({ providedIn: 'root' })
export class Here {
  /** The open conversation, or nothing when no session is on screen. */
  readonly open = signal<Summary | undefined>(undefined);

  /**
   * The session the route names. [open] waits on `/api/state`, so which screen
   * you are on is a fact about the URL.
   */
  readonly at = signal<string | undefined>(undefined);

  /**
   * What this conversation is about, when a sentence has been written for it.
   * Beside the summary, not on it: gists cover the transcripts on disk too.
   */
  readonly gist = signal<Gist | undefined>(undefined);

  /** How much is left of this conversation's list. Keyed by conversation, as [gist] is. */
  readonly tasks = signal<TaskCount | undefined>(undefined);
}
