import { Component, computed, inject, signal } from '@angular/core';
import { MAT_BOTTOM_SHEET_DATA } from '@angular/material/bottom-sheet';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';

import { ConsoleApi } from './console-api';
import { Parsed, Step } from './models';
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
 * How deeply a step may be indented before indenting stops helping.
 *
 * ⚠ **A phone is 412px wide and the words are monospace.** The corpus nests
 * twice at most, so three levels covers everything real; past that the indent
 * would eat the column the command is written in, which is the one thing on this
 * sheet that must stay readable. Depth is still shown — it moves to the label —
 * so nothing is hidden, only un-indented.
 */
const DEEPEST_INDENT = 3;

/**
 * One `Bash` command, as written and as read.
 *
 * ⚠ **The raw text is above the parse and not behind a switch.** A toggle would
 * hide exactly the comparison this exists for: the interesting failures are the
 * ones where a command parses perfectly, classifies correctly, names a real
 * path — and still attributes nothing. Seeing why takes both halves at once.
 *
 * ⚠ **Nothing here can run anything.** Every command on this sheet has already
 * run; the runner parses the text and executes none of it. Worth stating in the
 * one place a reader might mistake a description for an offer.
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
   * Why the parse could not be fetched, as distinct from a command the parser
   * could not read — which is [[Parsed.error]] and is an answer, not a failure.
   *
   * dev-lint: allow-sticky-error the sheet asks once, on the way up, and has no
   * second attempt to clear it.
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
      /** The words, rejoined. Rejoined rather than sent as one string, because
       *  what is shown is the argv *after* expansion — which is the whole point
       *  — and that no longer exists as text anywhere. */
      words: step.argv.join(' '),
    })),
  );

  /**
   * The one line of context every relative path in the parse depends on.
   *
   * Taken from the first step that names a directory rather than shown per
   * step: a `cd` inside the command changes it for what follows, and repeating
   * it on every row would bury the one time it changes under the times it does
   * not. Rows whose directory differs from this say so themselves.
   */
  protected readonly against = computed(() => this.parsed()?.steps.find((s) => s.cwd)?.cwd);

  /** Whether this step resolved against somewhere other than the sheet's own
   *  heading — a `cd`, a subshell, or a directory that became unknowable. */
  protected moved(step: Step): boolean {
    return step.cwd !== this.against();
  }

  /**
   * What to say about a condition, in the fewest words that stay true.
   *
   * `always` is most of the corpus and gets no label at all: a chip on every row
   * is a chip nobody reads, and the two that matter stop standing out.
   */
  protected condition(step: Step): string | undefined {
    if (step.reached === 'on-success') return 'only if what precedes it worked';
    if (step.reached === 'sometimes') return 'sometimes — the text cannot say when';
    return undefined;
  }

  /** How many uses the whole parse found, for the summary line. */
  protected readonly counted = computed(() => {
    const uses = (this.parsed()?.steps ?? []).flatMap((step) => step.uses ?? []);
    return {
      steps: this.parsed()?.steps.length ?? 0,
      reads: uses.filter((used) => !used.write).length,
      writes: uses.filter((used) => used.write).length,
      /** ⚠ **Counted, not filtered.** A use the outcome cannot confirm is still
       *  shown — it is the most interesting row on the sheet — and this is how
       *  the reader knows how many of them there are. */
      unproven: uses.filter((used) => !used.certain).length,
    };
  });
}
