import { Component, DestroyRef, OnDestroy, effect, inject, input, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';
import { MatProgressBarModule } from '@angular/material/progress-bar';

import { ConsoleApi } from './console-api';
import { reason } from './errors';
import { Foreground } from './foreground';
import { Entry, SessionEvent, Summary } from './models';
import { fold } from './transcript';

/** One session: what it has done, and the way to say something to it. */
@Component({
  selector: 'app-session-view',
  templateUrl: './session-view.html',
  styleUrl: './session-view.scss',
  imports: [
    FormsModule,
    MatButtonModule,
    MatIconModule,
    MatFormFieldModule,
    MatInputModule,
    MatProgressBarModule,
  ],
})
export class SessionView implements OnDestroy {
  /** Bound from the route, so a session is a link. */
  readonly id = input.required<string>();

  private api = inject(ConsoleApi);
  private foreground = inject(Foreground);
  private until = inject(DestroyRef);
  private close?: () => void;
  private poll?: ReturnType<typeof setInterval>;

  readonly entries = signal<Entry[]>([]);
  readonly session = signal<Summary | undefined>(undefined);
  readonly trouble = signal('');
  readonly sending = signal(false);
  /** Thinking is folded away by default: it is the longest thing on the page and
   *  the least often what the reader came for. */
  readonly showThinking = signal(false);
  readonly text = signal('');

  constructor() {
    effect(() => {
      const id = this.id();
      this.close?.();
      this.entries.set([]);
      this.close = this.api.follow(
        id,
        (event) => this.take(event),
        // A reconnect replays the transcript from the top, so the old copy is
        // dropped rather than appended to — otherwise every dropped connection
        // would double the page.
        () => this.entries.set([]),
      );
      this.refresh();
      this.poll ??= setInterval(() => this.refresh(), 5000);
    });
    // The poll does not run while the phone is away, so the header facts on
    // screen when it comes back are as old as the pocket it was in. The
    // transcript below them heals itself — EventSource reconnects and replays
    // from the top — and these totals have nothing that would.
    this.foreground.onReturn(() => this.refresh(), this.until);
  }

  ngOnDestroy(): void {
    this.close?.();
    if (this.poll) clearInterval(this.poll);
  }

  /** The header facts — cost, turns, whether it is working — come from the
   *  summary rather than the stream, because they are totals and a client that
   *  reconnected mid-session has not seen every event that built them. */
  private refresh(): void {
    this.api.state().subscribe({
      next: (state) => this.session.set(state.sessions.find((s) => s.id === this.id())),
      error: (err: unknown) => this.trouble.set(`cannot reach the runner: ${reason(err)}`),
    });
  }

  private take(event: SessionEvent): void {
    this.entries.update((entries) => [...fold(entries, event)]);
  }

  send(): void {
    const text = this.text().trim();
    if (!text || this.sending()) return;
    this.sending.set(true);
    this.api.send(this.id(), text).subscribe({
      next: (summary) => {
        this.sending.set(false);
        this.text.set('');
        this.session.set(summary);
      },
      error: (err: unknown) => {
        this.sending.set(false);
        this.trouble.set(reason(err));
      },
    });
  }

  /** Approve or refuse one question.
   *
   *  The verdict is not written into the entry here — the runner echoes an
   *  `answered` event to every listener, and letting that do it means a second
   *  window showing the same session stops offering a decision that was already
   *  taken.
   */
  decide(entry: Entry, allow: boolean): void {
    if (!entry.ask || entry.allowed !== undefined) return;
    this.api.decide(this.id(), entry.ask, allow).subscribe({
      error: (err: unknown) => this.trouble.set(reason(err)),
    });
  }

  stop(): void {
    this.api.stop(this.id()).subscribe({
      next: (summary) => this.session.set(summary),
      error: (err: unknown) => this.trouble.set(reason(err)),
    });
  }

  visible(): Entry[] {
    const all = this.entries();
    return this.showThinking() ? all : all.filter((entry) => entry.kind !== 'thought');
  }

  thoughts(): number {
    return this.entries().filter((entry) => entry.kind === 'thought').length;
  }
}
