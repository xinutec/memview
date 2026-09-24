import { Component, ElementRef, inject, input, output } from '@angular/core';

import { EntryRow } from './entry-row';
import { Folding } from './folding';
import { type Change, type Entry, type ToolCall } from './models';
import { RunRow } from './run-row';
import { type Block } from './transcript';

/**
 * A transcript's rows: runs of tool calls folded into one row, and every other
 * entry as its own. The host is the list, so the page that scrolls it keeps its
 * own scrolling; an "earlier" row is projected in at the top.
 */
@Component({
  selector: 'ol[app-transcript]',
  templateUrl: './transcript-list.html',
  imports: [EntryRow, RunRow],
})
export class TranscriptList {
  readonly blocks = input.required<readonly Block[]>();
  readonly folding = input.required<Folding>();
  readonly pictureAt = input.required<(name: string) => string>();
  /** A clock the parent ticks while something is running. */
  readonly now = input(0);
  /** The session has stopped reading, so a queued message will not be. */
  readonly deaf = input(false);
  /** The picture currently shown full size, by name. */
  readonly full = input<string | undefined>(undefined);
  /** Who a prompt came from, as its row says. */
  readonly asker = input('you');

  /** The list element, which the page scrolls. */
  readonly host = inject<ElementRef<HTMLElement>>(ElementRef);

  readonly parse = output<ToolCall>();
  readonly diff = output<Change>();
  readonly enlarge = output<string>();

  protected shown(block: Block): readonly Entry[] {
    if (block.kind === 'one') return [block.entry];
    return this.folding().opensRun(block.key) ? block.entries : [];
  }
}
