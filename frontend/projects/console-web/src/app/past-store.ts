import { Injectable, inject, signal } from '@angular/core';

import { ConsoleApi } from './console-api';
import { Conversation } from './models';

/**
 * The conversations on disk that could be picked up again. Root-provided so the
 * list survives navigation rather than blanking on every return. Fetched again
 * whenever somebody is looking: `busy` is a snapshot, and the list once said
 * *in use* minutes after the session had gone.
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
      // Deliberately silent: this list is an extra way in, and a banner for it would
      // sit above live sessions working perfectly well.
      error: () => (this.asking = false),
    });
  }
}
