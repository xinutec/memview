import { Component, computed, inject, input } from '@angular/core';
import { MatButtonModule } from '@angular/material/button';

import { Asks } from './asks';
import type { Questioned } from './models';
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

  protected readonly asks = inject(Asks);
  private readonly ask = computed(() => this.entry().ask);

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
  protected readonly words = computed(() => this.asks.words(this.ask()));
  protected readonly replying = computed(() => this.asks.replying(this.ask()));
  protected readonly ready = computed(() => this.asks.ready(this.entry()));
  /** A single single-choice question answers on the tap; anything else needs a send. */
  protected readonly needsSending = computed(() => {
    const questions = this.entry().questions ?? [];
    return this.replying() || questions.length > 1 || (questions[0]?.multiSelect ?? false);
  });

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
