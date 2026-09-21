import { Component, computed, input } from '@angular/core';
import { MatIconModule } from '@angular/material/icon';

/**
 * What to tell the reader about the link to the runner. Never two at once — see
 * [[notice]] for the ranking.
 *
 * Not a place for "this is a kept copy": that is a fact about what is on screen
 * rather than about the link, and a dropped stream would outrank it.
 */
export type Notice =
  /** Something the reader pressed failed. Said on the press: they are watching. */
  | { readonly kind: 'acting'; readonly what: string }
  /** The runner has stopped answering the poll. See [[Roster]]. */
  | { readonly kind: 'runner'; readonly what: string }
  /** This conversation's stream has stopped arriving. See [[SessionStore]]. */
  | { readonly kind: 'stream' };

/**
 * The one thing worth saying, of everything that might be. An action the reader
 * just took beats a state, being the one they caused; the runner beats the
 * stream, a stopped stream being what an unreachable runner looks like.
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

/** The one banner. Angular Material has none, so this is the card, done once. */
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
