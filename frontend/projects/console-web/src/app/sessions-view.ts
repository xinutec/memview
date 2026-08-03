import { Component, DestroyRef, inject, signal } from '@angular/core';
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
import { Foreground } from './foreground';
import { Conversation, Overview, Summary } from './models';
import { modelName } from './model';
import { modeIcon, modeIsLoud, modeTitle } from './modes';
import { costMatters } from './budget';
import { Updates } from './updates';
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
  private updates = inject(Updates);
  private router = inject(Router);
  private pastStore = inject(PastStore);
  private foreground = inject(Foreground);
  private until = inject(DestroyRef);

  readonly state = signal<Overview | undefined>(undefined);
  readonly trouble = signal('');
  /**
   * The last poll's verdict on whether the Mac is reachable — its own signal,
   * cleared by the next poll that succeeds.
   *
   * ⚠ Separate from [trouble] because the two have opposite lifetimes. A failed
   * action is news that stays true until it is retried; a failed poll is a
   * snapshot that the next poll five seconds later supersedes. Sharing one
   * signal meant a single missed poll — a phone freezing, a socket dropped mid
   * flight — left "cannot reach the runner" on screen for as long as the page
   * was open, over a console that had been answering the whole time.
   */
  readonly unreachable = signal('');
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
    setInterval(() => {
      this.load();
      // Only while the list is open, which is the only time its `in use` marks
      // are being read — and the time they must be true, since a conversation
      // just closed is the one about to be picked up.
      if (this.showPast()) this.pastStore.load();
    }, 5000);
    // Once at the start, to know whether there is anything to offer at all.
    this.pastStore.load();
    // And whenever the phone comes back, because the poll above did not run
    // while it was away — see [[Foreground]].
    this.foreground.onReturn(() => {
      this.load();
      this.pastStore.load();
    }, this.until);
  }

  /** Show or hide the earlier conversations, refreshing them on the way open. */
  togglePast(): void {
    this.showPast.set(!this.showPast());
    if (this.showPast()) this.pastStore.load();
  }

  private load(): void {
    this.api.state().subscribe({
      next: (state) => {
        this.state.set(state);
        this.updates.saw(state.bundle);
        this.unreachable.set('');
        if (!this.dir()) this.dir.set(state.repos[0] ?? state.dirs[0] ?? '');
      },
      error: (err: unknown) => this.unreachable.set(`cannot reach the runner: ${reason(err)}`),
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
    // The row is the whole control, so the guard belongs here rather than only
    // in the styling: a busy conversation tapped anyway would reach the runner,
    // be refused, and put an error on screen for doing what the row offered.
    if (conversation.busy) return;
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

  /** What this session may do without asking, in the CLI's own words. */
  modeOf(session: Summary): string | undefined {
    return modeTitle(session.mode);
  }

  /** What the model is called, rather than the id it is shipped under. */
  modelOf(session: Summary): string | undefined {
    return modelName(session.model);
  }

  /** The icon standing for it, since the card has no room for the name. */
  modeIconOf(session: Summary): string | undefined {
    return modeIcon(session.mode);
  }

  /** Whether that mode is one the CLI itself colours as an error. */
  modeLoud(session: Summary): boolean {
    return modeIsLoud(session.mode);
  }

  /** Whether this session's cost is worth showing at all. See budget.ts. */
  costMatters(session: Summary): boolean {
    return costMatters(session.limit);
  }

  cost(session: Summary): string {
    return session.cost_usd < 0.01
      ? `$${session.cost_usd.toFixed(4)}`
      : `$${session.cost_usd.toFixed(2)}`;
  }

  since(session: Summary): string {
    const minutes = Math.max(0, Math.round(Date.now() / 1000 - session.started) / 60);
    if (minutes < 1) return 'just now';
    if (minutes < 60) return `${Math.round(minutes)}m ago`;
    return `${Math.round(minutes / 60)}h ago`;
  }
}
