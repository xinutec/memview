import { Component, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatCardModule } from '@angular/material/card';
import { MatChipsModule } from '@angular/material/chips';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';
import { MatListModule } from '@angular/material/list';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatSelectModule } from '@angular/material/select';
import { Router, RouterLink } from '@angular/router';

import { ConsoleApi } from './console-api';
import { reason } from './errors';
import { Conversation, Overview, Summary } from './models';
import { PastStore } from './past-store';

/** Every session this console owns, and the way to start another. */
@Component({
  selector: 'app-sessions-view',
  templateUrl: './sessions-view.html',
  styleUrl: './sessions-view.scss',
  imports: [
    RouterLink,
    FormsModule,
    MatCardModule,
    MatButtonModule,
    MatIconModule,
    MatFormFieldModule,
    MatInputModule,
    MatSelectModule,
    MatChipsModule,
    MatListModule,
    MatProgressBarModule,
  ],
})
export class SessionsView {
  private api = inject(ConsoleApi);
  private router = inject(Router);
  private pastStore = inject(PastStore);

  readonly state = signal<Overview | undefined>(undefined);
  readonly trouble = signal('');
  readonly starting = signal(false);
  readonly dir = signal('');
  readonly prompt = signal('');
  /** Conversations on disk, newest first. Held in a root store so opening a
   *  session and coming back does not blank the list — see [[PastStore]]. */
  readonly past = this.pastStore.conversations;
  readonly showPast = signal(false);

  constructor() {
    this.load();
    // The list is a snapshot of processes, and a session started from another
    // window — or one that just ended — should not need a manual refresh to
    // appear. Cheap: one small request.
    setInterval(() => this.load(), 5000);
    // Once, not on the poll: reading a dozen transcripts off disk to answer it is
    // cheap but not free, and the answer changes when a session ends, not every
    // five seconds.
    this.pastStore.load();
  }

  private load(): void {
    this.api.state().subscribe({
      next: (state) => {
        this.state.set(state);
        if (!this.dir()) this.dir.set(state.repos[0] ?? state.dirs[0] ?? '');
      },
      error: (err: unknown) => this.trouble.set(`cannot reach the runner: ${reason(err)}`),
    });
  }

  start(): void {
    this.open(this.dir().trim(), undefined);
  }

  /**
   * Pick up a conversation where it left off.
   *
   * ⚠ Only safe for one that has ended. Nothing stops two processes appending to
   * a transcript, and the console cannot see a `claude` running in a terminal —
   * so the warning in the template is the whole of the guard.
   */
  resume(conversation: Conversation): void {
    this.open(conversation.dir, conversation.id);
  }

  private open(dir: string, resume?: string): void {
    if (!dir || this.starting()) return;
    this.starting.set(true);
    this.trouble.set('');
    this.api.start(dir, this.prompt().trim(), resume).subscribe({
      next: (session) => {
        this.starting.set(false);
        this.prompt.set('');
        void this.router.navigate(['/s', session.id]);
      },
      error: (err: unknown) => {
        this.starting.set(false);
        this.trouble.set(reason(err));
      },
    });
  }

  /** How long ago, from a millisecond timestamp. */
  ago(at: number): string {
    const minutes = Math.max(0, (Date.now() - at) / 60000);
    if (minutes < 1) return 'just now';
    if (minutes < 60) return `${Math.round(minutes)}m ago`;
    if (minutes < 60 * 24) return `${Math.round(minutes / 60)}h ago`;
    return `${Math.round(minutes / 1440)}d ago`;
  }

  /** Megabytes, which is the only sense of size worth showing. */
  size(bytes: number): string {
    return `${Math.max(1, Math.round(bytes / 1048576))} MB`;
  }

  /** The last path element, which is what a repository is called. */
  place(session: Summary): string {
    return session.dir.split('/').filter(Boolean).pop() ?? session.dir;
  }

  cost(session: Summary): string {
    return session.cost_usd < 0.01 ? `$${session.cost_usd.toFixed(4)}` : `$${session.cost_usd.toFixed(2)}`;
  }

  since(session: Summary): string {
    const minutes = Math.max(0, Math.round(Date.now() / 1000 - session.started) / 60);
    if (minutes < 1) return 'just now';
    if (minutes < 60) return `${Math.round(minutes)}m ago`;
    return `${Math.round(minutes / 60)}h ago`;
  }
}
