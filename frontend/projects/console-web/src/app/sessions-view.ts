import { Component, DestroyRef, computed, inject, signal } from '@angular/core';
import { NgTemplateOutlet } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatCardModule } from '@angular/material/card';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';
import { MatProgressBarModule } from '@angular/material/progress-bar';
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

/**
 * One line of the list — a session this console is running, or a conversation
 * sitting on disk that could be picked up.
 *
 * ⚠ **One list, because two were a lie about what is there.** The conversations
 * used to live behind a collapsed card, so a console driving a dozen sessions
 * showed the three it had started this run and hid the rest behind a count. What
 * somebody wants from this page is *everything that exists and which of it is
 * awake* — so the two sources are merged and the answer to "is it on" is carried
 * by the row rather than by which list it was filed under.
 */
interface Row {
  readonly id: string;
  /** What to call it: its own name, or the repository it runs in. */
  readonly title: string;
  /** Whether a name was found, so the fallback can read as an identifier. */
  readonly named: boolean;
  /** Present when the console knows the process — running or finished. */
  readonly live?: Summary;
  /** Present when there is a transcript to resume. */
  readonly past?: Conversation;
  /** Working, waiting, idle, off — see [RANK]. */
  readonly rank: number;
  /** When it last did anything, in milliseconds, for ordering within a rank. */
  readonly at: number;
}

/**
 * The order the list is read in.
 *
 * Working first, because that is the question the page is opened to answer.
 * Blocked second: it needs an answer, but it is not going anywhere, whereas a
 * working session is the one whose output is arriving now. Everything awake sits
 * above everything that is not, and the ones that are off keep their places
 * relative to each other by when they were last touched.
 */
const RANK = { working: 0, waiting: 1, idle: 2, off: 3 } as const;

/** Every session this console owns, and the way to start another. */
@Component({
  selector: 'app-sessions-view',
  templateUrl: './sessions-view.html',
  styleUrl: './sessions-view.scss',
  imports: [
    RouterLink,
    NgTemplateOutlet,
    FormsModule,
    MatCardModule,
    MatButtonModule,
    MatIconModule,
    MatFormFieldModule,
    MatInputModule,
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

  /**
   * Everything there is, awake first.
   *
   * ⚠ **Deduped by id, and the running process wins.** A session the console
   * started also has a transcript on disk, so both sources describe it — and the
   * process is the one that knows what it is doing, what it was asked and how
   * much it has cost. The disk copy of the same conversation would otherwise
   * appear a second time, greyed, directly below the live one.
   */
  readonly rows = computed<Row[]>(() => {
    const rows: Row[] = [];
    const seen = new Set<string>();
    for (const session of this.state()?.sessions ?? []) {
      seen.add(session.id);
      rows.push({
        id: session.id,
        title: session.name ?? this.place(session),
        named: !!session.name,
        live: session,
        rank: !session.alive
          ? RANK.off
          : session.busy
            ? RANK.working
            : session.waiting
              ? RANK.waiting
              : RANK.idle,
        // Seconds on the wire here, milliseconds on a conversation.
        at: session.started * 1000,
      });
    }
    for (const conversation of this.past()) {
      if (seen.has(conversation.id)) continue;
      rows.push({
        id: conversation.id,
        title: conversation.name ?? conversation.id.slice(0, 8),
        named: !!conversation.name,
        past: conversation,
        rank: RANK.off,
        at: conversation.modified,
      });
    }
    // Newest last-activity first inside a rank, so the top of each group is the
    // one most recently in play.
    return rows.sort((a, b) => a.rank - b.rank || b.at - a.at);
  });

  /** Whether anything on the list is held by a process the console cannot see,
   *  which is the only reason the warning about it is worth the space. */
  readonly anyInUse = computed(() => this.rows().some((row) => row.past?.busy));

  constructor() {
    this.load();
    // The list is a snapshot of processes, and a session started from another
    // window — or one that just ended — should not need a manual refresh to
    // appear. Cheap: one small request.
    setInterval(() => {
      this.load();
      // Unconditional now that the conversations are in the list rather than
      // behind a disclosure: they are on screen whenever this page is, so `busy`
      // has to be as fresh as the sessions beside it. A conversation just closed
      // is the one about to be picked up.
      this.pastStore.load();
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
  resume(conversation: Conversation | undefined): void {
    // The row is the whole control, so the guard belongs here rather than only
    // in the styling: a busy conversation tapped anyway would reach the runner,
    // be refused, and put an error on screen for doing what the row offered.
    if (!conversation || conversation.busy) return;
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

  /** The last path element, which is what a repository is called.
   *
   *  Takes the directory rather than a session, because a conversation on disk
   *  has one too and the answer is the same question about the same string. */
  place(what: { dir: string }): string {
    return what.dir.split('/').filter(Boolean).pop() ?? what.dir;
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
