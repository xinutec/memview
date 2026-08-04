import { Injectable, signal } from '@angular/core';

import { Summary } from './models';

/**
 * Which conversation the reader is looking at, for the parts of the shell that
 * sit above the router and cannot ask it.
 *
 * The whole summary rather than the four fields the toolbar reads, because the
 * toolbar is also the way into the details sheet and that shows nearly all of it
 * — a narrower type here meant copying fields across one at a time, and the
 * first thing to go missing was the one the sheet exists to show.
 *
 * A signal on a root service rather than a route parameter because none of this
 * is in the URL: the URL carries a session id, and the name, the mode and
 * everything else come from the runner.
 */
@Injectable({ providedIn: 'root' })
export class Here {
  /** The open conversation, or nothing when no session is on screen. */
  readonly open = signal<Summary | undefined>(undefined);

  /**
   * The session the *route* names, which is known at once.
   *
   * ⚠ **[open] is not the answer to "am I in a session".** It carries a summary
   * from the runner, so it stays empty until `/api/state` replies — and on a cold
   * launch the phone reopens straight inside a conversation, so for one round
   * trip the shell believed it was on the list and drew the list's toolbar. It
   * was a visible flash of the wrong screen's bar: `console` and a terminal
   * glyph, then the whole row swapping for the session's.
   *
   * Which screen you are on is a fact about the URL and owes nothing to the
   * network. Set by [[SessionView]] from its route input, and cleared when it
   * goes.
   */
  readonly at = signal<string | undefined>(undefined);
}
