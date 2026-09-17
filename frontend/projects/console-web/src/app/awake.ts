import { DOCUMENT, Injectable, inject, signal } from '@angular/core';
import { ScreenAwake } from '@xinutec/ui-harness/awake';

import { Telemetry } from './telemetry';

/**
 * The Angular binding for keeping the phone's screen on; the lock and the
 * remembering are shared in `@xinutec/ui-harness/awake`, which is built by plain
 * tsc and cannot carry a decorated class. Wanted here more than anywhere:
 * waking the screen by hand is also the gesture that stops the transcript
 * following (see [[Following]]).
 */
@Injectable({ providedIn: 'root' })
export class Awake {
  private readonly telemetry = inject(Telemetry);
  private readonly on = signal(false);

  private readonly core = new ScreenAwake(inject(DOCUMENT), {
    // Named for the app: the console and the memory viewer are one origin, and a
    // choice made while watching a session is not one about reading the corpus.
    key: 'console.awake',
    onChange: (on) => this.on.set(on),
    // Nothing on screen — the button going back to hollow is the report; the
    // refusal goes to [[Telemetry.note]] so it is not lost.
    onRefused: (why) => this.telemetry.note('awake-refused', why),
  });

  /** Whether the screen is being kept on. What the toolbar draws. */
  readonly lit = this.on.asReadonly();

  /**
   * Whether this browser can do it at all — a control that cannot work should not
   * take room from the session's name.
   */
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
