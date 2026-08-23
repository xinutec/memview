import { Component, DestroyRef, computed, inject, signal } from '@angular/core';
import { NgTemplateOutlet } from '@angular/common';
import { MatButtonModule } from '@angular/material/button';
import { MatCardModule } from '@angular/material/card';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatBottomSheet } from '@angular/material/bottom-sheet';
import { Router, RouterLink } from '@angular/router';

import { ConsoleApi } from './console-api';
import { Dismiss } from './dismiss';
import { reason } from './errors';
import { Reach } from './reach';
import { Foreground } from './foreground';
import { Conversation, Held, Overview, Summary, TaskCount } from './models';
import { modelName } from './model';
import { modeIcon, modeIsLoud, modeTitle } from './modes';
import { placeOf, titleOf } from './naming';
import { fullness, tokens } from './tokens';
import { Updates } from './updates';
import { ReadingStrip } from './reading-strip';
import { UsageStrip } from './usage-strip';
import { PastStore } from './past-store';
import { StartSheet } from './start-sheet';

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
  /**
   * How full its context is, as `496k / 1M` — the same fact the session's own
   * header shows, and read the same way for a row that is running and a row
   * that is not. Undefined when nothing has said.
   *
   * Computed here rather than in a template method: a binding is re-evaluated
   * on every change-detection pass, and this one is a fact about the row that
   * changes when the row does.
   */
  readonly context?: string;
  /**
   * What this conversation is about, in a sentence — and the moment it was
   * written, because it is a description of a thing that keeps changing.
   *
   * ⚠ **Inference, and drawn as such.** Every other field here is read off a
   * file or a process; this one is a model's reading of the transcript. See
   * `console/src/gist.rs`.
   */
  readonly gist?: {
    readonly text: string;
    readonly at: number;
    /** A few words for the same conversation, when the model gave some. Offered
     *  by the rename sheet and applied by nobody but the person reading it. */
    readonly name?: string;
  };
  /**
   * How much of its own task list is left, when it keeps one.
   *
   * ⚠ **Present for a conversation that is not running, too.** The list is on
   * disk beside the transcript and outlives the process — so a session finished
   * yesterday can still say it left three things open, which is exactly the row
   * somebody scanning this page is looking for.
   */
  readonly tasks?: TaskCount;
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
 *
 * ⚠ **Work left running is its own rank, above idle.** Within a rank the order
 * is last activity, and a background task is silent until it finishes — so a
 * session with two of them running sank at exactly the rate of one that had
 * finished for the day, and was found below a conversation that had genuinely
 * stopped. It sits under `waiting` because that one is blocked on *you*: this
 * needs nothing, but it is not done either, and it is the row to find when the
 * notification lands.
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
    UsageStrip,
    ReadingStrip,
  ],
})
export class SessionsView {
  private api = inject(ConsoleApi);
  private updates = inject(Updates);
  private router = inject(Router);
  private sheet = inject(MatBottomSheet);
  private dismiss = inject(Dismiss);
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
  /** How patient the banner above is. See [[Reach]]. */
  private readonly reach = new Reach();
  readonly starting = signal(false);
  /** Conversations on disk, newest first. Held in a root store so opening a
   *  session and coming back does not blank the list — see [[PastStore]]. */
  readonly past = this.pastStore.conversations;

  /**
   * What the card's tally says out loud — the hover and the accessible name,
   * which are the same sentence because the question is the same one.
   *
   * A sentence rather than template concatenation: with the leftovers it has
   * two clauses, either of which can be the only one.
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
   * pile.
   *
   * ⚠ **In the service's order, not re-sorted here.** It decides who is loaded
   * in one place, so `task sessions`, the app and this cannot disagree about it.
   * A holder with nothing at all is already left out upstream.
   */
  readonly elsewhere = computed<readonly Held[]>(() => this.state()?.tasks?.elsewhere ?? []);

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
    const gists = this.state()?.gists ?? {};
    // Keyed by conversation like the sentences, and read the same way for both
    // halves of the list — see [[Overview.tasks]].
    const tasks = this.state()?.tasks?.sessions ?? {};
    for (const session of this.state()?.sessions ?? []) {
      seen.add(session.id);
      rows.push({
        id: session.id,
        title: titleOf(session),
        named: !!session.name,
        live: session,
        context: fullness(session.context, session.window),
        gist: gists[session.id],
        tasks: tasks[session.id],
        rank: !session.alive
          ? RANK.off
          : session.busy
            ? RANK.working
            : session.waiting
              ? RANK.waiting
              : session.background
                ? RANK.background
                : RANK.idle,
        // ⚠ **Last activity, not when the process started.** `started` is when
        // this console picked the session up; a conversation that has run all
        // day reported `13h ago` while its transcript was four seconds old.
        // Falls back to `started` only for a session with no transcript yet,
        // which has nothing else to be dated by. Milliseconds either way —
        // `started` is the one quantity here that arrives in seconds.
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
        // ⚠ **Named, where a running session's is not.** A transcript records
        // how full each request was and never how big the window is (see
        // [[Conversation.context]]), so this is `340k` where the row above says
        // `340k / 1M` — and a bare `340k` beside `12 MB` could be anything. The
        // denominator is what carries the unit when there is one.
        context: conversation.context ? `${tokens(conversation.context)} tokens` : undefined,
        // Where it earns its keep: a name you have not opened in a week is a
        // word, and this says what the week's work was.
        gist: gists[conversation.id],
        tasks: tasks[conversation.id],
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
    //
    // ⚠ **Stopped when the page goes, and it was not.** This component is
    // rebuilt on every navigation back to the list, so a poll left running
    // accumulated one timer per visit — twenty trips through a session and the
    // phone was asking for `/api/state` and `/api/past` twenty times every five
    // seconds. The second of those walks every project directory and reads the
    // tail of every transcript on the Mac, so the leak was not only the phone's.
    // `SessionView` has always cleared its own; this is the one that did not.
    const poll = setInterval(() => {
      this.load();
      // Unconditional now that the conversations are in the list rather than
      // behind a disclosure: they are on screen whenever this page is, so `busy`
      // has to be as fresh as the sessions beside it. A conversation just closed
      // is the one about to be picked up.
      this.pastStore.load();
    }, 5000);
    this.until.onDestroy(() => clearInterval(poll));
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
        this.unreachable.set(this.reach.answered());
      },
      // Only once it has outlived a poll — see [[Reach]] for the measurement.
      error: (err: unknown) =>
        this.unreachable.set(this.reach.failed(`cannot reach the runner: ${reason(err)}`)),
    });
  }

  /** Offer the form that starts one. See [[StartSheet]] for why it is a sheet. */
  add(): void {
    // ⚠ Wired into history like the details sheet, and this is the case that
    // matters most: the list is the root, so a back press with this open leaves
    // the app altogether. See [[Dismiss]].
    this.dismiss.onBack(
      this.sheet.open(StartSheet, {
        data: { repos: this.state()?.repos ?? [], common: this.commonest() },
        panelClass: 'start-sheet',
      }),
    );
  }

  /**
   * The directory this machine's conversations actually run in.
   *
   * ⚠ **Not the first repository alphabetically**, which is what the field used
   * to open on — a real directory, but one nothing had ever been started in. It
   * looked deliberate and was not: `repos` is `read_dir` sorted, so the default
   * was whichever name happened to come first.
   *
   * Counted over the live sessions and the transcripts together, because a
   * console that has just started holds no sessions at all and the conversations
   * on disk are the whole of what it knows.
   */
  private commonest(): string | undefined {
    const seen = new Map<string, number>();
    for (const dir of [
      ...(this.state()?.sessions ?? []).map((s) => s.dir),
      ...this.past().map((c) => c.dir),
    ]) {
      if (dir) seen.set(dir, (seen.get(dir) ?? 0) + 1);
    }
    // Ties broken by whichever was counted first, which is the live sessions —
    // what is running now beats what once ran.
    return [...seen.entries()].sort((a, b) => b[1] - a[1])[0]?.[0];
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

  /**
   * Pick a conversation up, with no opening instruction.
   *
   * ⚠ **It used to send whatever was typed in the start form**, which shared
   * this page with the list. The form is a sheet now, so there is no such field
   * to read — and resuming with nothing said is the better default anyway: the
   * conversation already has a subject, and the composer is right there.
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

  /** How long ago, from a millisecond timestamp. */
  ago(at: number): string {
    const minutes = Math.max(0, (Date.now() - at) / 60000);
    if (minutes < 1) return 'just now';
    if (minutes < 60) return `${Math.round(minutes)}m ago`;
    if (minutes < 60 * 24) return `${Math.round(minutes / 60)}h ago`;
    return `${Math.round(minutes / 1440)}d ago`;
  }

  /** The last path element, which is what a repository is called.
   *
   *  Takes the directory rather than a session, because a conversation on disk
   *  has one too and the answer is the same question about the same string. */
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
