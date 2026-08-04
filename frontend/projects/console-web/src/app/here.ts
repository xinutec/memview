import { Injectable, signal } from '@angular/core';

/** The open conversation, as much of it as the shell above the router needs. */
export interface Open {
  id: string;
  /** What it calls itself. Absent until the runner has read it from the file. */
  name?: string;
  /** What it may do without asking; one of the modes in `modes.ts`. */
  mode?: string;
  alive: boolean;
}

/**
 * Which conversation the reader is looking at, for the parts of the shell that
 * sit above the router and cannot ask it.
 *
 * What the toolbar does with it is offer the things that can be done TO the
 * session — its permission mode, and stopping it — behind the overflow icon.
 * Naming it there is the page's job, not the shell's: the page's own heading is
 * pinned and says so already.
 *
 * A signal on a root service rather than a route parameter because none of this
 * is in the URL: the URL carries a session id, and the name and the mode come
 * from the runner.
 */
@Injectable({ providedIn: 'root' })
export class Here {
  /** The open conversation, or nothing when no session is on screen. */
  readonly open = signal<Open | undefined>(undefined);
}
