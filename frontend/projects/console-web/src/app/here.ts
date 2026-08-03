import { Injectable, computed, signal } from '@angular/core';

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
 * The toolbar says where the console is pointed. "this Mac" is true and says
 * nothing you did not know; once a session is open, the thing worth reading
 * there is *which agent you are talking to* — the console drives a dozen at once
 * and they differ only by name. That name is also the one control always on
 * screen, which is why pressing it opens the session's menu.
 *
 * A signal on a root service rather than a route parameter because none of this
 * is in the URL: the URL carries a session id, and the name and the mode come
 * from the runner.
 */
@Injectable({ providedIn: 'root' })
export class Here {
  /** The open conversation, or nothing when no session is on screen. */
  readonly open = signal<Open | undefined>(undefined);
  readonly name = computed(() => this.open()?.name);
}
