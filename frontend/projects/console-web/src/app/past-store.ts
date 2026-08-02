import { Injectable, inject, signal } from '@angular/core';

import { ConsoleApi } from './console-api';
import { Conversation } from './models';

/**
 * The conversations on disk that could be picked up again.
 *
 * Root-provided rather than held in the component, because the component is
 * destroyed and rebuilt every time you open a session and come back — and a list
 * that re-fetches on each return blanks in between, which reads as "there is
 * nothing to resume" for as long as the request takes. Held here it is fetched
 * once and survives the navigation.
 *
 * Refreshed only on request. Reading it means opening a dozen transcripts to find
 * the working directory each records, and the answer changes when a session ends
 * — not every few seconds like the live list beside it.
 */
@Injectable({ providedIn: 'root' })
export class PastStore {
  private api = inject(ConsoleApi);

  readonly conversations = signal<Conversation[]>([]);
  private asked = false;

  /** Fetch once. Later calls are ignored unless `force` says otherwise. */
  load(force = false): void {
    if (this.asked && !force) return;
    this.asked = true;
    this.api.past().subscribe({
      next: (conversations) => this.conversations.set(conversations),
      // Deliberately silent. This list is an extra way in, not the page's
      // purpose, and an error banner for it would sit above the live sessions
      // that are working perfectly well.
      error: () => this.asked = false,
    });
  }
}
