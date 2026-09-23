import { Component, DestroyRef, computed, inject, signal } from '@angular/core';
import { NgTemplateOutlet } from '@angular/common';
import { MatButtonModule } from '@angular/material/button';
import { MatCardModule } from '@angular/material/card';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatBottomSheet } from '@angular/material/bottom-sheet';
import { Router, RouterLink } from '@angular/router';

import { cacheStops, cacheUrgent, withinCacheHour } from './cache-heat';
import { since } from './since';
import { ConsoleApi } from './console-api';
import { Dismiss } from './dismiss';
import { reason } from './errors';
import { Roster } from './roster';
import { Foreground } from './foreground';
import { Conversation, Gist, Holder, Summary, TaskCount } from './models';
import { modelName } from './model';
import { modeIcon, modeIsLoud, modeTitle } from './modes';
import { Notice, NoticeBar, notice } from './notice';
import { placeOf, titleOf } from './naming';
import { fullness, tokens } from './tokens';
import { Updates } from './updates';
import { UsageStrip } from './usage-strip';
import { PastStore } from './past-store';
import { StartSheet } from './start-sheet';

/**
 * One line of the list — a session this console is running, or a conversation
 * on disk that could be picked up. One list: two of them hid a dozen
 * conversations behind a count, and the answer to "is it on" is carried by the
 * row.
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
  /**
   * Whether a prompt has been sent, and so whether there is a cache to lose: a
   * session fresh out of `/compact` has sent none.
   */
  readonly cached?: boolean;
  /**
   * How full its context is, as `496k / 1M`, read the same way for a running row
   * and a finished one. Undefined when nothing has said.
   */
  readonly context?: string;
  /** The same, 0–100, when the window is known. */
  readonly fill?: number;
  /**
   * What this conversation is about, in a sentence, and when it was written.
   * Inference, drawn as such — see `console/src/gist.rs`.
   */
  readonly gist?: Gist;
  /**
   * How much of its own task list is left. Present for a conversation that is
   * not running too: the list outlives the process.
   */
  readonly tasks?: TaskCount;
  /**
   * Whether something is written here and not sent — whether there is TEXT, not
   * an entry: a cleared draft stays as a tombstone.
   */
  readonly draft: boolean;
  /** Working, waiting, idle, off — see [RANK]. */
  readonly rank: number;
  /** When it last did anything, in milliseconds, for ordering within a rank. */
  readonly at: number;
}

/**
 * The order the list is read in. Working first, because that is the question
 * the page is opened to answer; blocked second — it needs an answer but is not
 * going anywhere. Work left running is its own rank above idle: a session with
 * two background tasks is silent until they finish, and sank like one that had
 * stopped for the day. Within a rank, last activity.
 */
const RANK = { working: 0, waiting: 1, background: 2, idle: 3, off: 4 } as const;

/** Every session this console owns, and the way to start another. */
@Component({
  selector: 'app-sessions-view',
  templateUrl: './sessions-view.html',
  styleUrl: './sessions-view.scss',
  imports: [
    RouterLink,
    NgTemplateOutlet,
    MatCardModule,
    MatButtonModule,
    MatIconModule,
    MatProgressBarModule,
    NoticeBar,
    UsageStrip,
  ],
})
export class SessionsView {
  private api = inject(ConsoleApi);
  private updates = inject(Updates);
  private router = inject(Router);
  private sheet = inject(MatBottomSheet);
  private dismiss = inject(Dismiss);
  private roster = inject(Roster);
  private pastStore = inject(PastStore);
  private foreground = inject(Foreground);
  private until = inject(DestroyRef);

  /** What the runner is holding. Polled once for the app — see [[Roster]]. */
  readonly state = this.roster.state;
  readonly trouble = signal('');
  /**
   * The one thing worth saying about the link. A failed action beats a failing
   * poll: the two have opposite lifetimes — a failed action stays true until it is
   * retried, a failed poll is superseded five seconds later — and it is the one
   * the reader just caused.
   */
  readonly notice = computed<Notice | undefined>(() =>
    notice({ acting: this.trouble(), runner: this.roster.notice() }),
  );
  readonly starting = signal(false);
  /**
   * Conversations on disk, newest first. In a root store so coming back does not
   * blank the list — see [[PastStore]].
   */
  readonly past = this.pastStore.conversations;

  /**
   * What the card's tally says out loud — the hover and the accessible name. A
   * sentence, since with leftovers it has two clauses, either of which can be the
   * only one.
   */
  protected tally(tasks: TaskCount): string {
    return [
      tasks.total ? `${tasks.open} of ${tasks.total} tasks still open` : '',
      tasks.stray ? `${tasks.stray} still in the built-in store, which nothing reads` : '',
    ]
      .filter(Boolean)
      .join('; ');
  }

  /**
   * Who is holding tasks who is not a conversation: Pippijn, and the unassigned
   * pile. In the service's order, so `task sessions`, the app and this agree.
   */
  readonly elsewhere = computed<readonly Holder[]>(() => this.state()?.tasks?.elsewhere ?? []);

  /**
   * Everything there is, awake first. Deduped by id, the running process winning:
   * it knows what it is doing, what it was asked and what it has cost.
   */
  readonly rows = computed<Row[]>(() => {
    const rows: Row[] = [];
    const seen = new Set<string>();
    const gists = this.state()?.gists ?? {};
    // Keyed by conversation, read the same way for both halves — see [[Overview.tasks]].
    const tasks = this.state()?.tasks?.sessions ?? {};
    // Unsent words, by session id — for transcripts on disk too, since a draft
    // outlives the process.
    const drafts = this.state()?.drafts ?? {};
    for (const session of this.state()?.sessions ?? []) {
      seen.add(session.id);
      rows.push({
        id: session.id,
        title: titleOf(session),
        named: !!session.name,
        live: session,
        context: fullness(session.context, session.window),
        fill:
          session.context && session.window
            ? Math.round((session.context / session.window) * 100)
            : undefined,
        // `context` is the last request's prompt size, so absent means nothing cached.
        cached: !!session.context,
        gist: gists[session.id],
        tasks: tasks[session.id],
        draft: !!drafts[session.id]?.text,
        rank: !session.alive
          ? RANK.off
          : session.busy
            ? RANK.working
            : session.waiting
              ? RANK.waiting
              : session.background
                ? RANK.background
                : RANK.idle,
        // Last activity, not when the process started: a conversation that ran all day
        // reported `13h ago` with a transcript four seconds old. `started` (seconds)
        // only for a session with no transcript yet.
        at: session.touched ?? session.started * 1000,
      });
    }
    for (const conversation of this.past()) {
      if (seen.has(conversation.id)) continue;
      rows.push({
        id: conversation.id,
        title: conversation.name ?? conversation.id.slice(0, 8),
        named: !!conversation.name,
        past: conversation,
        // Named, where a running session's is not: a transcript records how full each
        // request was and never the window, so a bare `340k` beside `12 MB` needs a unit.
        context: conversation.context ? `${tokens(conversation.context)} tokens` : undefined,
        // Where it earns its keep: a name you have not opened in a week is a
        // word, and this says what the week's work was.
        gist: gists[conversation.id],
        tasks: tasks[conversation.id],
        draft: !!drafts[conversation.id]?.text,
        rank: RANK.off,
        at: conversation.modified,
      });
    }
    // Newest last-activity first inside a rank.
    return rows.sort((a, b) => a.rank - b.rank || b.at - a.at);
  });

  /** Whether anything on the list is held by a process the console cannot see. */
  readonly anyInUse = computed(() => this.rows().some((row) => row.past?.busy));

  constructor() {
    // The roster does the asking; this says the list is being read, and stops when
    // the page goes.
    this.until.onDestroy(this.roster.follow());
    // The conversations on disk are this page's alone, so it keeps its own timer.
    // Stopped when the page goes: rebuilt on every navigation back, a poll left
    // running accumulated a timer per visit, each walking every transcript on the Mac.
    const poll = setInterval(() => this.pastStore.load(), 5000);
    this.until.onDestroy(() => clearInterval(poll));
    this.pastStore.load();
    // And whenever the phone comes back — see [[Foreground]].
    this.foreground.onReturn(() => {
      this.roster.ask();
      this.pastStore.load();
    }, this.until);
  }

  /** Offer the form that starts one. See [[StartSheet]] for why it is a sheet. */
  add(): void {
    // Wired into history: the list is the root, so a back press with this open
    // leaves the app altogether. See [[Dismiss]].
    this.dismiss.onBack(
      this.sheet.open(StartSheet, {
        data: { repos: this.state()?.repos ?? [], common: this.commonest() },
        panelClass: 'start-sheet',
      }),
    );
  }

  /**
   * The directory this machine's conversations actually run in — counted over
   * live sessions and transcripts together, since a console that has just started
   * holds no sessions. Not the first repository alphabetically, which is what the
   * field used to open on.
   */
  private commonest(): string | undefined {
    const seen = new Map<string, number>();
    for (const dir of [
      ...(this.state()?.sessions ?? []).map((s) => s.dir),
      ...this.past().map((c) => c.dir),
    ]) {
      if (dir) seen.set(dir, (seen.get(dir) ?? 0) + 1);
    }
    // Ties go to the live sessions, counted first.
    return [...seen.entries()].sort((a, b) => b[1] - a[1])[0]?.[0];
  }

  /**
   * Pick up a conversation where it left off. Only safe for one that has ended;
   * the console cannot see a `claude` in a terminal, so the template's warning
   * is the whole of the guard.
   */
  resume(conversation: Conversation | undefined): void {
    // The row is the whole control, so the guard belongs here: a busy conversation
    // tapped anyway would be refused with an error for doing what the row offered.
    if (!conversation || conversation.busy) return;
    this.open(conversation.dir, conversation.id);
  }

  /**
   * Pick a conversation up, with no opening instruction: it already has a
   * subject, and the composer is right there.
   */
  private open(dir: string, resume?: string): void {
    if (!dir || this.starting()) return;
    this.starting.set(true);
    this.trouble.set('');
    this.api.start(dir, '', resume).subscribe({
      next: (session) => {
        this.starting.set(false);
        void this.router.navigate(['/s', session.id]);
      },
      error: (err: unknown) => {
        this.starting.set(false);
        this.trouble.set(reason(err));
      },
    });
  }

  /** How long ago, from a millisecond timestamp. See [[since]]. */
  ago(at: number): string {
    return since(at);
  }

  /** Whether the prompt cache's hour is still running, and so whether to show it. */
  withinHour(at: number): boolean {
    return withinCacheHour(at);
  }

  /**
   * Where along the ramp this row sits. The two legs are mixed in the
   * stylesheet, so both ends follow light and dark.
   */
  cacheStyle(at: number): Record<string, number> {
    const { warm, hot } = cacheStops(at);
    return { '--warm': warm, '--hot': hot };
  }

  /** Whether the last ten minutes have started. */
  cacheIsUrgent(at: number): boolean {
    return cacheUrgent(at);
  }

  /** What the reddening clock means, for a title and a screen reader. */
  cacheSays(at: number): string {
    const left = Math.max(0, Math.round(60 - (Date.now() - at) / 60000));
    // What is LEFT, which is the number the reader acts on.
    return `${left}m left of the hour the prompt cache lasts`;
  }

  /** The last path element, which is what a repository is called. */
  place(what: { dir: string }): string {
    return placeOf(what.dir);
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
}
