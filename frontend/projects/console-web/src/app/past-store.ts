import { Injectable, inject, signal } from '@angular/core';

import { ConsoleApi } from './console-api';
import { Conversation } from './models';

/**
 * The conversations on disk that could be picked up again.
 *
 * Root-provided rather than held in the component, because the component is
 * destroyed and rebuilt every time you open a session and come back — and a list
 * that starts empty on each return reads as "there is nothing to resume" for as
 * long as the request takes. Held here it survives the navigation, and a refetch
 * replaces the list only once the answer arrives, so it never blanks.
 *
 * ⚠ Fetch again whenever somebody is looking. `busy` is a snapshot of what was
 * running when the runner was asked, and a conversation you close is one you are
 * about to want: fetched once, the list still said **in use** minutes after the
 * session it described had gone. Nothing about the list is expensive enough to
 * justify that — a dozen transcripts, a seek and a tail each.
 */
@Injectable({ providedIn: 'root' })
export class PastStore {
  private api = inject(ConsoleApi);

  readonly conversations = signal<Conversation[]>([]);
  private asking = false;

  /** Ask again, unless an earlier ask is still out. */
  load(): void {
    if (this.asking) return;
    this.asking = true;
    this.api.past().subscribe({
      next: (conversations) => {
        this.conversations.set(conversations);
        this.asking = false;
      },
      // Deliberately silent. This list is an extra way in, not the page's
      // purpose, and an error banner for it would sit above the live sessions
      // that are working perfectly well. The previous answer stays on screen.
      error: () => (this.asking = false),
    });
  }
}
