import { Component, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatCardModule } from '@angular/material/card';
import { MatChipsModule } from '@angular/material/chips';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatSelectModule } from '@angular/material/select';
import { Router, RouterLink } from '@angular/router';

import { ConsoleApi } from './console-api';
import { Overview, Summary } from './models';

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
    MatProgressBarModule,
  ],
})
export class SessionsView {
  private api = inject(ConsoleApi);
  private router = inject(Router);

  readonly state = signal<Overview | undefined>(undefined);
  readonly trouble = signal('');
  readonly starting = signal(false);
  readonly dir = signal('');
  readonly prompt = signal('');

  constructor() {
    this.load();
    // The list is a snapshot of processes, and a session started from another
    // window — or one that just ended — should not need a manual refresh to
    // appear. Cheap: one small request.
    setInterval(() => this.load(), 5000);
  }

  private load(): void {
    this.api.state().subscribe({
      next: (state) => {
        this.state.set(state);
        if (!this.dir()) this.dir.set(state.repos[0] ?? state.dirs[0] ?? '');
      },
      error: (err: unknown) => this.trouble.set(`cannot reach the runner: ${String(err)}`),
    });
  }

  start(): void {
    const dir = this.dir().trim();
    if (!dir || this.starting()) return;
    this.starting.set(true);
    this.trouble.set('');
    this.api.start(dir, this.prompt().trim()).subscribe({
      next: (session) => {
        this.starting.set(false);
        this.prompt.set('');
        void this.router.navigate(['/s', session.id]);
      },
      error: (err: { error?: string }) => {
        this.starting.set(false);
        this.trouble.set(err.error ?? 'could not start a session');
      },
    });
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
