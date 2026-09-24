import { Injectable, inject, signal } from '@angular/core';

import { ConsoleApi } from './console-api';
import { reason } from './errors';
import { Here } from './here';
import type { Identified, Questioned, Unanswered } from './models';
import { type Answers, type Notes, type Question, complete } from './questions';

/**
 * What has been tapped, typed and noted against the questions on screen.
 * Keyed by ask id, which is why this is not in the card: a re-seed builds new
 * entries and destroys every card, and the answers at risk are exactly the ones
 * tapped while the tunnel was dropping. The card reads its own ask out of this.
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
   * What went wrong sending an answer, if anything. Cleared when the next attempt
   * is made, not only on failure.
   */
  readonly trouble = signal('');

  answers(of: Identified): Answers {
    return this.chosen()[of.ask] ?? {};
  }

  notes(of: Identified): Notes {
    return this.noted()[of.ask] ?? {};
  }

  words(of: Identified): string {
    return this.said()[of.ask] ?? '';
  }

  /**
   * Whether this ask is being answered in words rather than by choice. The two
   * are alternatives: the CLI reports `response` before `answers` and only the
   * one it finds, so typing takes the card over and clearing the field hands it back.
   */
  replying(of: Identified): boolean {
    return this.words(of).trim() !== '';
  }

  /** Whether everything asked has been answered. The send button waits for it. */
  ready(entry: Questioned): boolean {
    return complete(entry.questions ?? [], this.answers(entry), this.notes(entry));
  }

  /** Whether a question's note field has been opened. It never closes on its own. */
  noteOpen(of: Identified, question: Question): boolean {
    return (
      this.noting().has(field(of, question)) || this.notes(of)[question.question] !== undefined
    );
  }

  openNote(of: Identified, question: Question): void {
    this.noting.update((open) => new Set([...open, field(of, question)]));
  }

  jot(of: Identified, question: Question, text: string): void {
    this.noted.update((all) => ({
      ...all,
      [of.ask]: { ...(all[of.ask] ?? {}), [question.question]: text },
    }));
  }

  say(of: Identified, text: string): void {
    this.said.update((all) => ({ ...all, [of.ask]: text }));
  }

  /**
   * Choose an option. One question with one answer sends on the tap — the shape
   * almost every question has, and on a phone one tap against two is answering
   * from the lock screen against putting it off. Anything else waits for [answer].
   */
  pick(entry: Unanswered, question: Question, label: string): void {
    if (this.replying(entry)) return;
    const questions = entry.questions ?? [];
    if (questions.length === 1 && !question.multiSelect) {
      this.send(entry, { [question.question]: label }, undefined, this.notes(entry));
      return;
    }
    this.chosen.update((all) => {
      const here = { ...(all[entry.ask] ?? {}) };
      if (question.multiSelect) {
        const had = here[question.question];
        const list = Array.isArray(had) ? had : [];
        here[question.question] = list.includes(label)
          ? list.filter((l) => l !== label)
          : [...list, label];
      } else {
        here[question.question] = label;
      }
      return { ...all, [entry.ask]: here };
    });
  }

  /** Send what has been chosen, or what has been typed instead of choosing. */
  answer(entry: Unanswered): void {
    if (this.replying(entry)) {
      // Words override the choices in the CLI, so nothing else goes with them.
      this.send(entry, undefined, this.words(entry).trim(), undefined);
      return;
    }
    if (!this.ready(entry)) return;
    this.send(entry, this.answers(entry), undefined, this.notes(entry));
  }

  /** Allow or refuse outright, for an ask that offers no questions. */
  decide(entry: Unanswered, allow: boolean): void {
    const at = this.here.at();
    if (!at) return;
    this.trouble.set('');
    this.api.decide(at, entry.ask, allow).subscribe({
      error: (err: unknown) => this.trouble.set(reason(err)),
    });
  }

  private send(entry: Unanswered, answers?: Answers, response?: string, notes?: Notes): void {
    const at = this.here.at();
    if (!at) return;
    this.trouble.set('');
    this.api.decide(at, entry.ask, true, undefined, answers, response, notes).subscribe({
      error: (err: unknown) => this.trouble.set(reason(err)),
    });
  }
}

/** Where one question's note is kept: the ask it belongs to, then the question. */
function field(of: Identified, question: Question): string {
  return `${of.ask}::${question.question}`;
}
