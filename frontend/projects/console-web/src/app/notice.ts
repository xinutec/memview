import { Component, computed, input } from '@angular/core';
import { MatIconModule } from '@angular/material/icon';

/**
 * What to tell the reader about the link to the runner. Three sources, and never
 * two of them at once — see [[notice]] for the ranking.
 *
 * Deliberately NOT a place for "this is a kept copy": that is a fact about what
 * is on screen rather than about the link, and folding it in here would let a
 * dropped stream outrank it and take away the more useful sentence.
 */
export type Notice =
  /** Something the reader pressed failed. Said on the press: they are watching. */
  | { readonly kind: 'acting'; readonly what: string }
  /** The runner has stopped answering the poll. See [[Roster]]. */
  | { readonly kind: 'runner'; readonly what: string }
  /** This conversation's stream has stopped arriving. See [[SessionStore]]. */
  | { readonly kind: 'stream' };

/**
 * The one thing worth saying, of everything that might be.
 *
 * Ranked, so that only ever one banner shows. An action the reader just took
 * beats a state, being the more specific and the one they caused. The runner
 * beats the stream: a stream that has stopped is what a runner that is not
 * answering LOOKS like, and naming the cause is more use than naming the symptom.
 */
export function notice(from: {
  readonly acting: string;
  readonly runner: Notice | undefined;
  readonly stream?: Notice | undefined;
}): Notice | undefined {
  if (from.acting) return { kind: 'acting', what: from.acting };
  return from.runner ?? from.stream;
}

/** A notice as it reads on screen. */
interface Worded {
  readonly icon: string;
  /** A state named in bold, for a notice that carries no message of its own. */
  readonly lead?: string;
  readonly what: string;
  /** Not an error: the transcript is behind, and that mends itself. */
  readonly quiet?: true;
}

function worded(notice: Notice): Worded {
  switch (notice.kind) {
    case 'acting':
      return { icon: 'error_outline', what: notice.what };
    case 'runner':
      return { icon: 'cloud_off', lead: 'Cannot reach the runner.', what: notice.what };
    case 'stream':
      return {
        icon: 'cloud_off',
        lead: 'Not live.',
        what: 'The stream dropped, so anything said since is not below yet.',
        quiet: true,
      };
  }
}

/**
 * The one banner. Angular Material has no banner component, so this is the
 * card the rest of the console draws by hand, done once.
 */
@Component({
  selector: 'app-notice',
  imports: [MatIconModule],
  templateUrl: './notice.html',
  styleUrl: './notice.scss',
})
export class NoticeBar {
  readonly notice = input.required<Notice>();
  protected readonly said = computed(() => worded(this.notice()));
}
