import { DOCUMENT, Injectable, inject, signal } from '@angular/core';
import { ScreenAwake } from '@xinutec/ui-harness/awake';

import { Telemetry } from './telemetry';

/**
 * The Angular binding for keeping the phone's screen on.
 *
 * The lock, the re-acquisition and the remembering are shared and tested once in
 * `@xinutec/ui-harness/awake` — every app in the fleet is also a WebView wrapper
 * with the same display timeout to argue with. What has to live here is the
 * framework binding, for the reason [[Telemetry]] gives: that package is built by
 * plain tsc, and a decorated class arriving without Ivy definitions fails a
 * production build on `JIT compiler unavailable`.
 *
 * It is wanted more here than in most of them. Watching a session work is done
 * with no hands for minutes at a time, and on a transcript that follows its own
 * end, waking the screen by hand is also the gesture that stops it following
 * (see [[Following]]) — so the timeout does not merely interrupt the reading, it
 * silently changes what the page will do next.
 */
@Injectable({ providedIn: 'root' })
export class Awake {
  private readonly telemetry = inject(Telemetry);
  private readonly on = signal(false);

  private readonly core = new ScreenAwake(inject(DOCUMENT), {
    // Named for the app rather than taking the shared default: the console and
    // the memory viewer are one origin behind the same sign-in, and a choice
    // made while watching a session is not one made about reading the corpus.
    key: 'console.awake',
    onChange: (on) => this.on.set(on),
    // Nothing is said on screen — the button going back to hollow is the whole
    // report a person needs, and a dialog over a toolbar is not. This is so the
    // refusal is not lost entirely; see [[Telemetry.note]].
    onRefused: (why) => this.telemetry.note('awake-refused', why),
  });

  /** Whether the screen is being kept on. What the toolbar draws. */
  readonly lit = this.on.asReadonly();

  /** Whether this browser can do it at all — a control that cannot work should
   *  not be on a 412px toolbar taking room from the session's name. */
  readonly possible = this.core.possible;

  /** Restore what was chosen last time. Called once from the app shell. */
  init(): void {
    this.core.start();
  }

  /** The button. */
  toggle(): void {
    this.core.toggle();
  }
}
