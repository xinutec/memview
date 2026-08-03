import { Injectable, signal } from '@angular/core';

/**
 * Which conversation the reader is looking at, for the parts of the shell that
 * sit above the router and cannot ask it.
 *
 * The toolbar says where the console is pointed. "this Mac" is true and says
 * nothing you did not know; once a session is open, the thing worth reading
 * there is *which agent you are talking to* — the console drives a dozen at
 * once and they differ only by name.
 *
 * A signal on a root service rather than a route parameter because the name is
 * not in the URL: the URL carries a session id, and the name comes from the
 * transcript that session is writing (`past::named`), which only the runner can
 * read.
 */
@Injectable({ providedIn: 'root' })
export class Here {
  /** The open conversation's name, or nothing when none is open. */
  readonly name = signal<string | undefined>(undefined);
}
