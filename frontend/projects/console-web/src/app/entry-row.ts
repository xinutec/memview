import { Component, computed, input, output } from '@angular/core';
import { MatButtonModule } from '@angular/material/button';
import { MatIconModule } from '@angular/material/icon';
import { RouterLink } from '@angular/router';

import { AskCard } from './ask-card';
import { Clock } from './clock';
import { Coloured } from './coloured';
import { Folding } from './folding';
import { Lasted } from './lasted';
import { type Change, type ToolCall, type Entry, pending } from './models';
import { fetchedAt, thumbnailAt } from './picture';
import { PICTURE, Rendered } from './rendered';

/**
 * One line of the transcript. An `<li>` so the list stays a list; the class is
 * what it is drawn as, which for a tool call awaiting permission is the ask.
 */
@Component({
  selector: 'li[app-entry]',
  templateUrl: './entry-row.html',
  styleUrl: './entry-row.scss',
  host: { '[class]': 'drawn()', class: 'entry' },
  imports: [AskCard, Clock, Coloured, Lasted, MatButtonModule, MatIconModule, Rendered, RouterLink],
})
export class EntryRow {
  readonly entry = input.required<Entry>();
  readonly folding = input.required<Folding>();
  /** The session has stopped reading, so a queued message will not be. */
  readonly deaf = input(false);
  /** A clock the parent ticks while something is running. */
  readonly now = input(0);
  /** The picture currently shown full size, by name. */
  readonly full = input<string | undefined>(undefined);
  readonly pictureAt = input.required<(name: string) => string>();
  /** Who a prompt came from, as its row says. */
  readonly asker = input('you');

  readonly parse = output<ToolCall>();
  readonly diff = output<Change>();
  readonly enlarge = output<string>();

  protected readonly drawn = computed(() => (pending(this.entry()) ? 'ask' : this.entry().kind));
  protected readonly pending = pending;
  protected readonly PICTURE = PICTURE;
  protected readonly fetchedAt = fetchedAt;
  protected readonly thumbnailAt = thumbnailAt;

  protected runningFor(entry: ToolCall): number | undefined {
    if (entry.unrecorded || entry.at === undefined) return undefined;
    return this.now() - entry.at;
  }

  protected parseable(entry: ToolCall): boolean {
    return entry.tool === 'Bash' && !!entry.text.trim();
  }
}
