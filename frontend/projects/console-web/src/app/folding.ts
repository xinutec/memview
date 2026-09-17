import { signal } from '@angular/core';

import type { ToolCall } from './models';

/**
 * Which tool results are unfolded, and which runs of calls are open.
 *
 * Keyed by the call's id rather than the entry: an entry is replaced when its
 * result arrives, and what was unfolded should stay so.
 */
export class Folding {
  private readonly opened = signal<ReadonlySet<string>>(new Set());
  private readonly runs = signal<Record<string, boolean>>({});

  shows(entry: ToolCall): boolean {
    return this.opened().has(keyOf(entry));
  }

  unfold(entry: ToolCall): void {
    const key = keyOf(entry);
    this.opened.update((open) => {
      const next = new Set(open);
      if (!next.delete(key)) next.add(key);
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

function keyOf(entry: ToolCall): string {
  return entry.call ?? `${entry.at ?? 0}:${entry.text}`;
}
