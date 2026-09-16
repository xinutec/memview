import { Component, computed, inject, input } from '@angular/core';
import { MatButtonModule } from '@angular/material/button';

import { Asks } from './asks';
import type { Entry } from './models';
import { type Question, choiceOf } from './questions';

/**
 * A permission question, as a card in the transcript.
 *
 * ⚠ **It owns no state.** The transcript's rows are tracked by object identity,
 * so a re-seed destroys every card — and what would go with them are the options
 * tapped while the tunnel was dropping. [[Asks]] keys them by ask id, where they
 * outlive the card, and this reads its own out of there.
 */
@Component({
  selector: 'app-ask-card',
  templateUrl: './ask-card.html',
  styleUrl: './ask-card.scss',
  imports: [MatButtonModule],
})
export class AskCard {
  readonly entry = input.required<Entry>();

  protected readonly asks = inject(Asks);

  private readonly ask = computed(() => this.entry().ask ?? '');

  /** What was picked, for the row that records it. Empty when there is nothing
   *  to say — a refusal, or any tool that is not a question. */
  protected readonly choice = computed(() =>
    this.entry().allowed ? choiceOf(this.entry().reply) : '',
  );

  /**
   * How the card reads once it has been answered.
   *
   * ⚠ **Until the session acts on it, this is a claim about the PIPE.**
   * `Answered` is pushed once the decision has been written and flushed, which
   * is not the same as the CLI having read it — and against a session that has
   * stopped reading, the old wording reported the answer as delivered and
   * accepted while the session stayed blocked on the same question. `health`
   * showed a green *answered* for thirty-one minutes (memview #122). See
   * [[Entry.settling]].
   */
  protected readonly verdict = computed(() => {
    const entry = this.entry();
    if (entry.settling) return 'sent — not taken up yet';
    if (!entry.questions) return entry.allowed ? 'allowed' : 'refused';
    if (!entry.allowed) return 'skipped';
    return entry.reply?.response?.trim() ? 'replied' : 'answered';
  });

  protected readonly words = computed(() => this.asks.words(this.ask()));
  protected readonly replying = computed(() => this.asks.replying(this.ask()));
  protected readonly ready = computed(() => this.asks.ready(this.entry()));

  /** Whether the button that sends is worth showing at all. */
  protected readonly needsSending = computed(() => {
    const questions = this.entry().questions ?? [];
    return this.replying() || questions.length > 1 || (questions[0]?.multiSelect ?? false);
  });

  /** Whether this option is currently chosen — what the button shows as pressed. */
  protected picked(question: Question, label: string): boolean {
    const chosen = this.asks.answers(this.ask())[question.question];
    return Array.isArray(chosen) ? chosen.includes(label) : chosen === label;
  }

  protected note(question: Question): string {
    return this.asks.notes(this.ask())[question.question] ?? '';
  }

  protected notable(question: Question): boolean {
    return this.asks.noteOpen(this.ask(), question);
  }

  protected pick(question: Question, label: string): void {
    this.asks.pick(this.entry(), question, label);
  }

  protected jot(question: Question, text: string): void {
    this.asks.jot(this.ask(), question, text);
  }

  protected addNote(question: Question): void {
    this.asks.openNote(this.ask(), question);
  }

  protected say(text: string): void {
    this.asks.say(this.ask(), text);
  }

  protected answer(): void {
    this.asks.answer(this.entry());
  }

  protected decide(allow: boolean): void {
    this.asks.decide(this.entry(), allow);
  }
}
