import { Component, computed, inject, signal } from '@angular/core';
import { MAT_BOTTOM_SHEET_DATA, MatBottomSheetRef } from '@angular/material/bottom-sheet';
import { MatButtonModule } from '@angular/material/button';
import { MatButtonToggleModule } from '@angular/material/button-toggle';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';

import { ConsoleApi } from './console-api';
import { Landmark, Mark } from './models';
import { reason } from './errors';

/** What the sheet is opened with. */
export interface Where {
  readonly session: string;
}

/** How one kind of landmark reads and draws. */
export interface Signpost {
  readonly icon: string;
  /** Shown in place of the text when there is none — a compaction is a place,
   *  not a thing said. */
  readonly instead: string;
}

/**
 * Distinct icons, because the kinds are what make the list scannable: grouped by
 * day these run to dozens of rows of the reader's own words, and the two that
 * are not words — a picture, a cut — are the ones people navigate by.
 */
const SIGNPOST: Record<Mark, Signpost> = {
  prompt: { icon: 'chat_bubble_outline', instead: 'something said' },
  command: { icon: 'terminal', instead: 'a command' },
  shown: { icon: 'image', instead: 'a picture' },
  compacted: { icon: 'content_cut', instead: 'the conversation was cut here' },
};

const UNKNOWN: Signpost = { icon: 'place', instead: 'a place in the conversation' };

export function signpostOf(kind: Mark): Signpost {
  return SIGNPOST[kind] ?? UNKNOWN;
}

/** One day's worth, newest day first and newest thing first within it. */
export interface Day {
  /** How the day reads — "Today", "Yesterday", or the date. */
  readonly title: string;
  readonly marks: readonly Landmark[];
}

/**
 * Group landmarks into days, newest first — this is a list of places to go back
 * to, and this afternoon's is wanted far more often than March's. Anything
 * undated is kept, in one group at the end.
 */
export function byDay(marks: readonly Landmark[], now: number): Day[] {
  const days = new Map<string, Landmark[]>();
  const undated: Landmark[] = [];
  for (const mark of [...marks].reverse()) {
    if (mark.when === undefined) {
      undated.push(mark);
      continue;
    }
    const key = new Date(mark.when).toDateString();
    days.set(key, [...(days.get(key) ?? []), mark]);
  }
  const today = new Date(now).toDateString();
  const yesterday = new Date(now - 86_400_000).toDateString();
  const named = (key: string): string => {
    if (key === today) return 'Today';
    if (key === yesterday) return 'Yesterday';
    return new Date(key).toLocaleDateString(undefined, {
      weekday: 'short',
      day: 'numeric',
      month: 'short',
    });
  };
  const grouped: Day[] = [...days.entries()].map(([key, held]) => ({
    title: named(key),
    marks: held,
  }));
  if (undated.length > 0) grouped.push({ title: 'Undated', marks: undated });
  return grouped;
}

/** The kinds a filter can be set to, in the order the toggles read. */
export const FILTERS: readonly Mark[] = ['prompt', 'shown', 'command', 'compacted'];

/**
 * Everywhere in this conversation worth going back to. Paging cannot reach the
 * past — a transcript comes back 400 events at a time, so an hour ago is a
 * hundred taps away. What a person remembers is what they said, what they sent
 * and where the conversation was cut.
 */
@Component({
  selector: 'app-jump-sheet',
  templateUrl: './jump-sheet.html',
  styleUrl: './jump-sheet.scss',
  imports: [MatButtonModule, MatButtonToggleModule, MatIconModule, MatProgressBarModule],
})
export class JumpSheet {
  private api = inject(ConsoleApi);
  private ref = inject<MatBottomSheetRef<JumpSheet, number>>(MatBottomSheetRef);
  protected readonly where = inject<Where>(MAT_BOTTOM_SHEET_DATA);

  protected readonly all = signal<Landmark[] | undefined>(undefined);
  /**
   * Why the list could not be read.
   *
   * dev-lint: allow-sticky-error read once on the way up, with no retry offered.
   */
  protected readonly trouble = signal('');
  /** Which kinds are shown. All of them until somebody narrows it. */
  protected readonly kinds = signal<Mark[]>([...FILTERS]);
  protected readonly filters = FILTERS;
  /**
   * The toggles' icons, by kind. A lookup rather than a call: a template method
   * runs on every change-detection pass (DL-ANGULAR-TEMPLATE-METHOD-CALL).
   */
  protected readonly icons: Record<Mark, string> = {
    prompt: SIGNPOST.prompt.icon,
    command: SIGNPOST.command.icon,
    shown: SIGNPOST.shown.icon,
    compacted: SIGNPOST.compacted.icon,
  };

  protected readonly days = computed(() =>
    byDay(
      (this.all() ?? []).filter((mark) => this.kinds().includes(mark.kind)),
      Date.now(),
    ),
  );
  protected readonly empty = computed(() => (this.all() ?? []).length === 0);

  constructor() {
    this.api.landmarks(this.where.session).subscribe({
      next: (found) => this.all.set(found),
      error: (failure: unknown) => {
        this.all.set([]);
        this.trouble.set(reason(failure));
      },
    });
  }

  protected signpostOf(mark: Landmark): Signpost {
    return signpostOf(mark.kind);
  }

  /** The offset goes back to the view, which is what owns the transcript. */
  protected go(mark: Landmark): void {
    this.ref.dismiss(mark.at);
  }
}
