import { Component, computed, inject, signal } from '@angular/core';
import { MAT_BOTTOM_SHEET_DATA } from '@angular/material/bottom-sheet';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';

import { ConsoleApi } from './console-api';
import { Parsed, Line } from './models';
import { reason } from './errors';

/** What the sheet is opened with: the command as it was written, and how its
 *  call turned out. */
export interface About {
  readonly session: string;
  readonly command: string;
  /** `undefined` while the call is still running — see [[ConsoleApi.parse]]. */
  readonly ok?: boolean;
}

/**
 * How deeply a step may be indented before indenting stops helping: a phone is
 * 412px wide, the words are monospace, and the corpus nests twice at most.
 * Depth past that moves to the label.
 */
const DEEPEST_INDENT = 3;

/**
 * One `Bash` command, as written and as read. The raw text is above the parse,
 * not behind a switch: the interesting failures are commands that parse
 * perfectly and still attribute nothing, which takes both halves at once.
 * Nothing here can run anything.
 */
@Component({
  selector: 'app-parse-sheet',
  templateUrl: './parse-sheet.html',
  styleUrl: './parse-sheet.scss',
  imports: [MatIconModule, MatProgressBarModule],
})
export class ParseSheet {
  private api = inject(ConsoleApi);
  protected readonly about = inject<About>(MAT_BOTTOM_SHEET_DATA);

  protected readonly parsed = signal<Parsed | undefined>(undefined);
  /**
   * Why the parse could not be fetched — distinct from a command the parser could
   * not read, which is [[Parsed.error]] and an answer.
   *
   * dev-lint: allow-sticky-error the sheet asks once and has no second attempt.
   */
  protected readonly trouble = signal<string | undefined>(undefined);

  constructor() {
    this.api.parse(this.about.session, this.about.command, this.about.ok).subscribe({
      next: (parsed) => this.parsed.set(parsed),
      error: (wrong) => this.trouble.set(reason(wrong)),
    });
  }

  /** Every step, with what the template needs that JSON cannot carry. */
  protected readonly steps = computed(() =>
    (this.parsed()?.steps ?? []).map((step) => ({
      step,
      indent: Math.min(step.depth, DEEPEST_INDENT),
      /**
       * The words, rejoined: what is shown is the argv AFTER expansion, which exists
       * as text nowhere else.
       */
      words: step.argv.join(' '),
    })),
  );

  /**
   * The one line of context every relative path depends on, from the first step
   * that names a directory; rows whose directory differs say so themselves.
   */
  protected readonly against = computed(() => this.parsed()?.steps.find((s) => s.cwd)?.cwd);

  /** Whether this step resolved against somewhere other than the sheet's own
   *  heading — a `cd`, a subshell, or a directory that became unknowable. */
  protected moved(step: Line): boolean {
    return step.cwd !== this.against();
  }

  /**
   * What to say about a condition, in the fewest words. `always` is most of the
   * corpus and gets no label, so the two that matter stand out.
   */
  protected condition(step: Line): string | undefined {
    if (step.reached === 'on-success') return 'only if what precedes it worked';
    if (step.reached === 'sometimes') return 'sometimes — the text cannot say when';
    return undefined;
  }

  /**
   * The concept's sentence with each path cut to its leaf.
   *
   * Shortened HERE, never in `describe` (memview#1454): that returns the CONCEPT's
   * phrase, and the path is already carried by the use row and the footer. A
   * token at a time, so a greedy match cannot eat the words between subjects.
   *
   * This would be WRONG for a locus — `under /var/log` cut to `under log` loses
   * the claim — and is safe only because `Subject::Located` and `Subject::Bounded`
   * cannot reach `describe` today (`subjects_or_refuse` refuses both). Lift that
   * refusal and this must learn the difference first.
   */
  protected concise(concept: string): string {
    return concept
      .split(' ')
      .map((word) => {
        const cut = word.lastIndexOf('/');
        // Not a path, or a trailing slash whose leaf would be nothing.
        return cut <= 0 || cut === word.length - 1 ? word : word.slice(cut + 1);
      })
      .join(' ');
  }

  /** How many uses the whole parse found, for the summary line. */
  protected readonly counted = computed(() => {
    const uses = (this.parsed()?.steps ?? []).flatMap((step) => step.uses ?? []);
    return {
      steps: this.parsed()?.steps.length ?? 0,
      reads: uses.filter((used) => !used.write).length,
      writes: uses.filter((used) => used.write).length,
      /**
       * Counted, not filtered: a use the outcome cannot confirm is the most
       * interesting row on the sheet.
       */
      unproven: uses.filter((used) => !used.certain).length,
    };
  });
}
