import { signal } from '@angular/core';

import type { Entry } from './models';

/**
 * What the reader has opened in one reading of a conversation.
 *
 * ⚠ **Per view, deliberately, which is why this is a plain object and not an
 * injectable.** It is about this reading of the conversation rather than about
 * the conversation: leaving a session and coming back opens every result closed
 * again, which is the predictable answer. A root-scoped service would quietly
 * change that by outliving the page.
 *
 * Everything starts closed, failures included — a blob that expands itself moves
 * the page under somebody who is reading it, and the red mark on the row already
 * says which one to open.
 */
export class Folding {
  /** Tool results the reader has opened, by entry. */
  private readonly opened = signal(new Set<Entry>());

  /** What the reader has said about each run of calls, which beats the default. */
  private readonly runs = signal<Record<string, boolean>>({});

  /** Takes the entry, so it cannot be a `computed` — and is a set lookup, which
   *  is what makes it cheap enough to run for every row on every pass. */
  shows(entry: Entry): boolean {
    return this.opened().has(entry);
  }

  unfold(entry: Entry): void {
    this.opened.update((open) => {
      const next = new Set(open);
      if (!next.delete(entry)) next.add(entry);
      return next;
    });
  }

  opensRun(key: string): boolean {
    return this.runs()[key] ?? false;
  }

  toggleRun(key: string): void {
    const open = this.opensRun(key);
    this.runs.update((choice) => ({ ...choice, [key]: !open }));
  }
}
