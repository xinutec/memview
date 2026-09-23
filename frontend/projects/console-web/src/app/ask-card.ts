import { Component, computed, inject, input, output } from '@angular/core';
import { MatButtonModule } from '@angular/material/button';

import { Asks } from './asks';
import { type Change, type Questioned, pending } from './models';
import { type Question, choiceOf } from './questions';

/** A question the session is asking, or asked: the choices, and then the answer. */
@Component({
  selector: 'app-ask-card',
  templateUrl: './ask-card.html',
  styleUrl: './ask-card.scss',
  imports: [MatButtonModule],
})
export class AskCard {
  readonly entry = input.required<Questioned>();
  readonly diff = output<Change>();

  protected readonly asks = inject(Asks);
  /**
   * This question while it is still answerable, else null. The one place a card's
   * entry becomes something [[Asks]] will take — see `Unanswered`. The template
   * already hides the controls once a verdict is in; this is what makes that a
   * fact the compiler holds rather than a rule the markup remembers.
   */
  private readonly open = computed(() => {
    const entry = this.entry();
    return pending(entry) ? entry : null;
  });

  protected readonly choice = computed(() =>
    this.entry().allowed ? choiceOf(this.entry().reply) : '',
  );
  protected readonly verdict = computed(() => {
    const entry = this.entry();
    if (entry.settling) return 'sent — not taken up yet';
    if (!entry.questions) return entry.allowed ? 'allowed' : 'refused';
    if (!entry.allowed) return 'skipped';
    return entry.reply?.response?.trim() ? 'replied' : 'answered';
  });
  protected readonly words = computed(() => this.asks.words(this.entry()));
  protected readonly replying = computed(() => this.asks.replying(this.entry()));
  protected readonly ready = computed(() => this.asks.ready(this.entry()));
  /** A single single-choice question answers on the tap; anything else needs a send. */
  protected readonly needsSending = computed(() => {
    const questions = this.entry().questions ?? [];
    return this.replying() || questions.length > 1 || (questions[0]?.multiSelect ?? false);
  });

  protected picked(question: Question, label: string): boolean {
    const chosen = this.asks.answers(this.entry())[question.question];
    return Array.isArray(chosen) ? chosen.includes(label) : chosen === label;
  }

  protected note(question: Question): string {
    return this.asks.notes(this.entry())[question.question] ?? '';
  }

  protected notable(question: Question): boolean {
    return this.asks.noteOpen(this.entry(), question);
  }

  protected pick(question: Question, label: string): void {
    const open = this.open();
    if (open) this.asks.pick(open, question, label);
  }

  protected jot(question: Question, text: string): void {
    this.asks.jot(this.entry(), question, text);
  }

  protected addNote(question: Question): void {
    this.asks.openNote(this.entry(), question);
  }

  protected say(text: string): void {
    this.asks.say(this.entry(), text);
  }

  protected answer(): void {
    const open = this.open();
    if (open) this.asks.answer(open);
  }

  protected decide(allow: boolean): void {
    const open = this.open();
    if (open) this.asks.decide(open, allow);
  }
}
