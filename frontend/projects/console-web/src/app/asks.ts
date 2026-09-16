import { Injectable, inject, signal } from '@angular/core';

import { ConsoleApi } from './console-api';
import { reason } from './errors';
import { Here } from './here';
import type { Entry } from './models';
import { type Answers, type Notes, type Question, complete } from './questions';

/**
 * What has been tapped, typed and noted against the questions on screen.
 *
 * ⚠ **Keyed by ASK ID, and that is the whole reason this is not in the card.**
 * The transcript's rows are tracked by object identity, so a re-seed — which the
 * stream does on every reconnect it cannot resume — builds new entries and
 * destroys every card. State held in a card would go with them, and the answers
 * at risk are exactly the ones that matter: options tapped while the tunnel was
 * dropping. Held here, they outlive the card, the view and the route.
 *
 * The card reads its own ask out of this and calls back into it. Nothing is
 * drilled through the transcript as bindings.
 */
@Injectable({ providedIn: 'root' })
export class Asks {
  private api = inject(ConsoleApi);
  private here = inject(Here);

  private readonly chosen = signal<Record<string, Answers>>({});
  private readonly noted = signal<Record<string, Notes>>({});
  private readonly said = signal<Record<string, string>>({});
  /** Which note fields have been opened, as `<ask>::<question>`. */
  private readonly noting = signal<ReadonlySet<string>>(new Set());

  /**
   * What went wrong sending an answer, if anything.
   *
   * ⚠ **Cleared when the next attempt is made, not only on failure.** A message
   * left standing after a retry succeeds tells somebody their answer did not go
   * when it did.
   */
  readonly trouble = signal('');

  answers(ask: string): Answers {
    return this.chosen()[ask] ?? {};
  }

  notes(ask: string): Notes {
    return this.noted()[ask] ?? {};
  }

  words(ask: string): string {
    return this.said()[ask] ?? '';
  }

  /**
   * Whether this ask is being answered in words rather than by choice.
   *
   * ⚠ **The two are alternatives, not companions.** The CLI's result builder
   * tests `response` before `answers` and reports only the one it finds, so
   * words sent alongside a set of taps would throw the taps away without saying
   * so. Typing therefore takes the card over: the options go quiet, and clearing
   * the field hands it back.
   */
  replying(ask: string): boolean {
    return this.words(ask).trim() !== '';
  }

  /** Whether everything asked has been answered. The send button waits for it. */
  ready(entry: Entry): boolean {
    const ask = entry.ask ?? '';
    return complete(entry.questions ?? [], this.answers(ask), this.notes(ask));
  }

  /** Whether a question's note field has been opened. It never closes on its
   *  own: a field that vanished while it held words would be taking them away. */
  noteOpen(ask: string, question: Question): boolean {
    return (
      this.noting().has(`${ask}::${question.question}`) ||
      this.notes(ask)[question.question] !== undefined
    );
  }

  openNote(ask: string, question: Question): void {
    this.noting.update((open) => new Set([...open, `${ask}::${question.question}`]));
  }

  jot(ask: string, question: Question, text: string): void {
    this.noted.update((all) => ({
      ...all,
      [ask]: { ...(all[ask] ?? {}), [question.question]: text },
    }));
  }

  say(ask: string, text: string): void {
    this.said.update((all) => ({ ...all, [ask]: text }));
  }

  /**
   * Choose an option.
   *
   * **One question with one answer sends on the tap.** That is the shape almost
   * every question has, and on a phone the difference between one tap and two is
   * the difference between answering from the lock screen and putting it off.
   * Anything else — several questions, or one that takes several answers — has
   * no moment where the choice is obviously finished, so it waits for [answer].
   */
  pick(entry: Entry, question: Question, label: string): void {
    const ask = entry.ask;
    if (!ask || entry.allowed !== undefined || this.replying(ask)) return;
    const questions = entry.questions ?? [];
    if (questions.length === 1 && !question.multiSelect) {
      this.send(entry, { [question.question]: label }, undefined, this.notes(ask));
      return;
    }
    this.chosen.update((all) => {
      const here = { ...(all[ask] ?? {}) };
      if (question.multiSelect) {
        const had = here[question.question];
        const list = Array.isArray(had) ? had : [];
        here[question.question] = list.includes(label)
          ? list.filter((l) => l !== label)
          : [...list, label];
      } else {
        here[question.question] = label;
      }
      return { ...all, [ask]: here };
    });
  }

  /** Send what has been chosen, or what has been typed instead of choosing. */
  answer(entry: Entry): void {
    const ask = entry.ask;
    if (!ask || entry.allowed !== undefined) return;
    if (this.replying(ask)) {
      // Words override the choices in the CLI, so nothing else goes with them —
      // notes included, which would be qualifying an answer that is not sent.
      this.send(entry, undefined, this.words(ask).trim(), undefined);
      return;
    }
    if (!this.ready(entry)) return;
    this.send(entry, this.answers(ask), undefined, this.notes(ask));
  }

  /** Allow or refuse outright, for an ask that offers no questions. */
  decide(entry: Entry, allow: boolean): void {
    const at = this.here.at();
    if (!entry.ask || entry.allowed !== undefined || !at) return;
    this.trouble.set('');
    this.api.decide(at, entry.ask, allow).subscribe({
      error: (err: unknown) => this.trouble.set(reason(err)),
    });
  }

  private send(entry: Entry, answers?: Answers, response?: string, notes?: Notes): void {
    const at = this.here.at();
    if (!entry.ask || entry.allowed !== undefined || !at) return;
    this.trouble.set('');
    this.api.decide(at, entry.ask, true, undefined, answers, response, notes).subscribe({
      error: (err: unknown) => this.trouble.set(reason(err)),
    });
  }
}
