import {
  Component,
  DestroyRef,
  ElementRef,
  Injector,
  OnDestroy,
  afterNextRender,
  computed,
  effect,
  inject,
  input,
  signal,
  viewChild,
} from '@angular/core';
import { TextFieldModule } from '@angular/cdk/text-field';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';
import { MatProgressBarModule } from '@angular/material/progress-bar';

import { Clock } from './clock';
import { ConsoleApi } from './console-api';
import { reason } from './errors';
import { Foreground } from './foreground';
import { Entry, Summary } from './models';
import { modelName } from './model';
import { modeIcon, modeIsLoud, modeTitle } from './modes';
import { costMatters } from './budget';
import { Here } from './here';
import { Updates } from './updates';
import { Rendered } from './rendered';
import { Held, SessionStore } from './session-store';

/** One session: what it has done, and the way to say something to it. */
@Component({
  selector: 'app-session-view',
  templateUrl: './session-view.html',
  styleUrl: './session-view.scss',
  imports: [
    Clock,
    FormsModule,
    MatButtonModule,
    MatIconModule,
    MatFormFieldModule,
    MatInputModule,
    MatProgressBarModule,
    Rendered,
    TextFieldModule,
  ],
})
export class SessionView implements OnDestroy {
  /** Bound from the route, so a session is a link. */
  readonly id = input.required<string>();

  private api = inject(ConsoleApi);
  private updates = inject(Updates);
  private here = inject(Here);
  /** A newer build is downloaded and held. See `Updates` for why it waits. */
  readonly updateWaiting = this.updates.waiting;
  private store = inject(SessionStore);
  private foreground = inject(Foreground);
  private until = inject(DestroyRef);
  /** For `afterNextRender` outside an injection context — see [loadEarlier]. */
  private injector = inject(Injector);
  private poll?: ReturnType<typeof setInterval>;

  /** The transcript being read, which outlives this view — see [[SessionStore]]. */
  private readonly held = signal<Held | undefined>(undefined);
  /** The conversation on screen. Two signals deep on purpose: which transcript
   *  is being read changes when the route does, and its contents change with
   *  every event, and a `computed` over both is what tracks each of them. */
  readonly entries = computed<Entry[]>(() => this.held()?.entries() ?? []);
  /**
   * What the session is doing this second, straight off the stream.
   *
   * Not `session().busy`: that rides the five-second poll, so it lagged the
   * work it described and missed anything shorter than the interval.
   */
  readonly doing = computed(() => this.held()?.doing());
  /**
   * How full the context is, as `496k / 1M`, when the session has said.
   *
   * Shown so compaction can be seen coming rather than met. Nothing else
   * reports it: the CLI puts the counts on its result line and nowhere in the
   * transcript, so a resumed session says nothing until its first turn ends.
   */
  readonly context = computed(() => {
    const session = this.session();
    if (!session?.context) return undefined;
    // The window is declared on the result line and nowhere else, so a session
    // that has not finished a turn since it started knows how full it is but
    // not what it is full of. Showing the count alone beats showing nothing:
    // the number people watch for is the first one.
    if (!session.window) return tokens(session.context);
    return `${tokens(session.context)} / ${tokens(session.window)}`;
  });

  /** How many background tasks the harness has told us about and not closed. */
  readonly background = computed(() => this.held()?.background().length ?? 0);
  readonly session = signal<Summary | undefined>(undefined);

  /**
   * Whether this session's cost is worth showing at all. See budget.ts.
   *
   * A computed rather than a method because the template reads it: a method
   * body runs on every change-detection pass that reaches this component and
   * cannot cache (DL-ANGULAR-TEMPLATE-METHOD-CALL).
   */
  readonly showsCost = computed(() => costMatters(this.session()?.limit));

  /** What the session may do without asking, in the CLI's own words. */
  readonly mode = computed(() => modeTitle(this.session()?.mode));
  /** What the model is called, rather than the id it is shipped under. */
  readonly model = computed(() => modelName(this.session()?.model));
  /** The icon standing for it where there is no room for the name. */
  readonly modeIcon = computed(() => modeIcon(this.session()?.mode));
  /** Whether that mode is one the CLI itself colours as an error. */
  readonly loud = computed(() => modeIsLoud(this.session()?.mode));
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
  readonly sending = signal(false);
  readonly text = signal('');
  /** Whether anything older than what is on screen remains on disk.
   *
   *  The cursor is a byte offset into the transcript, so zero is the start of the
   *  file and this is simply "not at the beginning". */
  readonly more = computed(() => (this.held()?.cursor() ?? 0) > 0);
  readonly loading = signal(false);
  private scroller = viewChild<ElementRef<HTMLElement>>('scroller');

  constructor() {
    effect((onCleanup) => {
      const id = this.id();
      this.held.set(this.store.open(id));
      // Covers leaving this session for another one and leaving it for the list:
      // the effect is cleaned up before it re-runs and again when the view goes.
      // What is left behind is the transcript, which is the point.
      onCleanup(() => this.store.leave(id));
      this.refresh();
      this.poll ??= setInterval(() => this.refresh(), 5000);
    });
    // The poll does not run while the phone is away, so the header facts on
    // screen when it comes back are as old as the pocket it was in. The
    // transcript below them heals itself — EventSource reconnects and replays
    // from the top — and these totals have nothing that would.
    this.foreground.onReturn(() => this.refresh(), this.until);
    // The soft keyboard is the biggest layout change this page ever sees: it
    // takes something like half the screen, and the transcript is what gives way
    // — `interactive-widget=resizes-content` shrinks the viewport rather than
    // sliding the page up. Nothing moves the scroll position, so the newest
    // message ends up below the fold at the exact moment somebody is answering
    // it. `visualViewport` is what reports the keyboard; a window resize does
    // not fire for it.
    const viewport = window.visualViewport;
    if (viewport) {
      // A frame later: the height the browser reports during the event is the
      // one from before it.
      const settle = () => requestAnimationFrame(() => this.follow());
      viewport.addEventListener('resize', settle);
      this.until.onDestroy(() => viewport.removeEventListener('resize', settle));
    }
    // A frame after the entries change, not with them: `follow` reads a height
    // that does not exist until the browser has laid the new nodes out.
    // `afterRenderEffect` was the first thing tried here and never ran — proven
    // by the layout harness, which kept reporting scrollY 0.
    effect(() => {
      this.entries();
      requestAnimationFrame(() => this.follow());
    });
  }

  ngOnDestroy(): void {
    if (this.poll) clearInterval(this.poll);
    // Leaving the page leaves the conversation, so the toolbar stops claiming
    // to be inside one — otherwise the list of sessions is titled with whichever
    // one was open last.
    this.here.open.set(undefined);
  }

  /** The header facts — cost, turns, whether it is working — come from the
   *  summary rather than the stream, because they are totals and a client that
   *  reconnected mid-session has not seen every event that built them. */
  private refresh(): void {
    this.api.state().subscribe({
      next: (state) => {
        const mine = state.sessions.find((s) => s.id === this.id());
        this.session.set(mine);
        // The toolbar sits above the router and cannot see the route, so the
        // page that knows which conversation this is has to say so — and the
        // menu behind that name acts on what is set here.
        this.here.open.set(
          mine && { id: mine.id, name: mine.name, mode: mine.mode, alive: mine.alive },
        );
        this.updates.saw(state.bundle);
        this.unreachable.set('');
      },
      error: (err: unknown) => this.unreachable.set(`cannot reach the runner: ${reason(err)}`),
    });
  }

  /**
   * Keep the newest in view.
   *
   * ⚠ A transcript opened at the top, which for a resumed conversation means
   * opening a hundred turns behind the present. The newest message is what
   * anybody came for, and scrolling to it by hand every time is the sort of
   * thing that reads as the page being broken.
   *
   * Only while the reader is already at the bottom: yanking the view down while
   * somebody is reading back through the morning is worse than not following at
   * all.
   */
  private follow(): void {
    const box = this.scroller()?.nativeElement;
    if (!box) return;
    if (this.pinned || !this.settled) {
      box.scrollTop = box.scrollHeight;
      this.settled = true;
    }
  }

  /**
   * Whether the reader is at the newest message.
   *
   * ⚠ **Remembered as they scroll, not measured when it is wanted.** It was
   * measured, and the moment it is wanted includes the soft keyboard opening:
   * by then the transcript has already lost half its height, so the arithmetic
   * says "several hundred pixels from the bottom" about a reader who has not
   * moved — and the message they tapped the box to answer slides off the screen
   * exactly as they start typing.
   */
  private pinned = true;

  /** `NEAR` is the slack: a few lines, so a partly-scrolled view still counts as
   *  following rather than as having been left behind. */
  onScroll(): void {
    const box = this.scroller()?.nativeElement;
    if (!box) return;
    const NEAR = 120;
    this.pinned = box.scrollHeight - box.scrollTop - box.clientHeight < NEAR;
  }

  /** Whether the first render has happened; before it there is nothing to keep. */
  private settled = false;

  /**
   * The page before the one on screen.
   *
   * The transcript belongs to the store; what belongs here is the reader's
   * place, because only the view can measure it. On demand rather than eager:
   * the runner re-reads and re-parses the file to answer.
   */
  loadEarlier(): void {
    if (this.loading()) return;
    const box = this.scroller()?.nativeElement;
    // Measured before anything is written. A height read after a signal write is
    // the height from before that write — change detection is scheduled, not
    // performed — so a baseline taken below `loading.set(true)` would describe a
    // page that no longer exists by the time it is used.
    const before = box?.scrollHeight ?? 0;
    this.loading.set(true);
    this.store.earlier(this.id()).subscribe({
      next: () => {
        this.loading.set(false);
        this.trouble.set('');
        // Hold the reader's place, a frame later. Adding above somebody moves
        // what they were reading down the screen by the height of everything
        // new — and that height does not exist until the browser has laid the
        // new entries out, which is after this callback has returned.
        afterNextRender(
          () => {
            if (box) box.scrollTop += box.scrollHeight - before;
          },
          { injector: this.injector },
        );
      },
      error: (err: unknown) => {
        this.loading.set(false);
        this.trouble.set(reason(err));
      },
    });
  }

  send(): void {
    const text = this.text().trim();
    if (!text || this.sending()) return;
    this.sending.set(true);
    this.api.send(this.id(), text).subscribe({
      next: (summary) => {
        this.sending.set(false);
        this.trouble.set('');
        this.text.set('');
        this.session.set(summary);
      },
      error: (err: unknown) => {
        this.sending.set(false);
        this.trouble.set(reason(err));
      },
    });
  }

  /**
   * The tool results whose whole output is on screen.
   *
   * Held here rather than on the entry because it is about this reading of the
   * conversation rather than about the conversation — leaving a session and
   * coming back opens every result closed again, which is the predictable
   * answer. Everything starts closed, failures included: a blob that expands
   * itself moves the page under somebody who is reading it, and the red mark on
   * the row already says which one to open.
   */
  private readonly opened = signal(new Set<Entry>());

  /** Takes the entry, so it cannot be a `computed` — and is a set lookup, which
   *  is what makes it cheap enough to run for every row on every pass. */
  shows(entry: Entry): boolean {
    return this.opened().has(entry);
  }

  unfold(entry: Entry): void {
    this.opened.update((open) => {
      const next = new Set(open);
      if (!next.delete(entry)) next.add(entry);
      return next;
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
}

/** Tokens, at the precision a glance wants: `496k`, `1M`. */
function tokens(count: number): string {
  if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(count % 1_000_000 === 0 ? 0 : 1)}M`;
  return `${Math.round(count / 1000)}k`;
}
