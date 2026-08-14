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
  untracked,
  viewChild,
} from '@angular/core';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { MatBottomSheet } from '@angular/material/bottom-sheet';
import { TextFieldModule } from '@angular/cdk/text-field';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { NgTemplateOutlet } from '@angular/common';

import { Clock } from './clock';
import { Lasted } from './lasted';
import { ConsoleApi } from './console-api';
import { reason } from './errors';
import { Reach } from './reach';
import { Foreground } from './foreground';
import { Entry, Summary } from './models';
import { modelName } from './model';
import { modeIcon, modeIsLoud, modeTitle } from './modes';
import { Dismiss } from './dismiss';
import { Drafts } from './drafts';
import { Following, measure } from './following';
import { Here } from './here';
import { Updates } from './updates';
import { Rendered } from './rendered';
import { Picture, shrink, weight } from './picture';
import { Answers, Notes, Question, choiceOf, complete } from './questions';
import { Held, SessionStore } from './session-store';
import { ParseSheet } from './parse-sheet';
import { Block, blocks, ran } from './transcript';
import { Telemetry } from './telemetry';
import { fullness } from './tokens';

/** One session: what it has done, and the way to say something to it. */
@Component({
  selector: 'app-session-view',
  templateUrl: './session-view.html',
  styleUrl: './session-view.scss',
  imports: [
    Clock,
    Lasted,
    FormsModule,
    NgTemplateOutlet,
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
  private sheet = inject(MatBottomSheet);
  private dismiss = inject(Dismiss);
  private updates = inject(Updates);
  private here = inject(Here);
  /** A newer build is downloaded and held. See `Updates` for why it waits. */
  readonly updateWaiting = this.updates.waiting;
  private store = inject(SessionStore);
  /** What is written and not sent, which outlives this view — see [[Drafts]]. */
  private drafts = inject(Drafts);
  private telemetry = inject(Telemetry);
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
   *
   * ⚠ **Except before the stream has spoken at all**, where the poll is the only
   * one of the two that knows anything — see [[Held.spoken]]. A status is
   * announced when it changes, so a client that reconnects to a session already
   * working hears nothing about it until it stops; falling back for that window
   * alone keeps the lag out of every ordinary turn, where the stream answers
   * first and this is never consulted.
   */
  readonly doing = computed(
    () => this.held()?.doing() ?? (this.held()?.spoken() ? undefined : this.session()?.busy),
  );
  /**
   * Whether to draw the activity strip — which is not the same question as
   * [doing], and was answered with it until this was split out.
   *
   * ⚠ **Nothing is arriving while a question stands, whatever the status says.**
   * A status is announced when it CHANGES and asking is not a change, so a
   * session blocked on `can_use_tool` still reads `requesting`: it is not
   * requesting anything, it is waiting for you, and the card saying so is on the
   * screen. The word stays in the header, where it is the CLI's own report of
   * where the turn got to. The bar claims something is arriving *now*, which
   * beside a question holding everything up is untrue — and it cost the card the
   * room it needed, taking the three pixels that pushed the first option off the
   * top of a page with no way to scroll to it. That is how this was found.
   */
  readonly arriving = computed(() => (this.session()?.waiting ? undefined : this.doing()));
  /**
   * Whether a turn is running, from the runner's own observation.
   *
   * ⚠ **Separate from [doing], which is only what the CLI last narrated.** A
   * status is announced when it CHANGES, so a session working steadily for
   * minutes can have nothing standing — and `doing() ?? 'idle'` drew that as
   * idle, about a session running tools throughout (memview #112). See
   * `session::Summary::working`.
   */
  readonly inTurn = computed(() => this.session()?.working ?? false);
  /** The word for it when the CLI has narrated nothing. */
  readonly state = computed(() => (this.inTurn() ? 'working' : 'idle'));
  /**
   * Now, to the second, but only while something is still happening.
   *
   * ⚠ **A clock that ticks for ever is a change-detection pass every second, for
   * ever** — on a phone, over a transcript of two thousand entries, for a page
   * that is usually sitting still. So the interval is started by the first thing
   * that starts running and stopped by the last one that finishes; see the
   * effect in the constructor.
   */
  private readonly now = signal(Date.now());
  /**
   * How long the session has been working, in milliseconds, or nothing when it
   * is not. See [[SessionStore]]'s `since` for what the clock runs from.
   */
  readonly working = computed(() => {
    const since = this.held()?.since();
    return since === undefined ? undefined : this.now() - since;
  });
  /** How long a call has been running. Takes the entry, so it cannot be a
   *  `computed` — and it is arithmetic on two numbers already to hand. */
  protected ranFor(entry: Entry): number | undefined {
    if (entry.unrecorded) return undefined;
    return entry.at === undefined ? undefined : this.now() - entry.at;
  }
  /** The oldest call still running in a folded run, which is the one the summary
   *  row reports: a run is as slow as the thing holding it up. */
  protected runningFor(block: Block & { kind: 'tools' }): number | undefined {
    const oldest = block.entries
      .filter((entry) => entry.ok === undefined && !entry.unrecorded && entry.at !== undefined)
      .map((entry) => entry.at ?? 0)
      .sort((a, b) => a - b)[0];
    return oldest === undefined ? undefined : this.now() - oldest;
  }
  /**
   * How full the context is, as `496k / 1M`, when the session has said.
   *
   * Shown so compaction can be seen coming rather than met. Formatted where the
   * list formats the same fact — see [[fullness]], and the row in
   * `sessions-view` that reads it for a conversation that is not running.
   */
  readonly context = computed(() => fullness(this.session()?.context, this.session()?.window));

  /**
   * How many background tasks the harness has told us about and not closed.
   *
   * ⚠ **The runner's count, not one kept here.** This page used to derive it
   * from its own event stream, which meant two answers to one question the
   * moment the list started showing it: the page's reset whenever the transcript
   * was re-seeded, the runner's did not, so a reload inside a session showed `0`
   * against a card saying `1`. One source, and it is the one that survives a
   * reload. The cost is that it rides the five-second poll rather than the
   * stream — which for work that runs for minutes is not a cost.
   */
  readonly background = computed(() => this.session()?.background ?? 0);

  /**
   * The background calls by name, for the strip.
   *
   * Falls back to the bare count when the runner has sent none — an older
   * runner, or a session whose calls detached before it learned to name them.
   * The strip then says what it used to say rather than nothing.
   */
  readonly running = computed(() => this.session()?.running ?? []);

  /**
   * Whether the process behind this conversation is gone.
   *
   * ⚠ **Not "there is no session" — the two are different and only one of them
   * means the past tense.** A page that has not loaded yet also has no
   * `alive`, and wording a strip in the past tense on that would date work
   * that is very much in flight. Undefined therefore reads as `false`.
   */
  readonly ended = computed(() => {
    const session = this.session();
    return !!session && !session.alive;
  });
  readonly session = signal<Summary | undefined>(undefined);

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
  /** How patient the banner above is. See [[Reach]]. */
  private readonly reach = new Reach();
  /** A restart of a session that stopped reading is in flight. See [[revive]]. */
  readonly reviving = signal(false);
  readonly sending = signal(false);
  readonly text = signal('');
  /**
   * A picture chosen and scaled, waiting to go with the next message.
   *
   * Held rather than sent on choosing, because the words about a screenshot are
   * the point of sending it — "this is what I meant by ragged" — and a picture
   * that left the moment it was picked would have to be explained in a second
   * message the model reads after it.
   */
  readonly picture = signal<Picture | undefined>(undefined);
  /** What went wrong choosing one — too large, not an image, a phone that
   *  refused. On the composer rather than in the transcript: it is about the
   *  thing being written, not about the conversation. */
  readonly pictureTrouble = signal('');
  /**
   * Options tapped but not yet sent, by control-request id and then by question.
   *
   * Keyed by the *ask* rather than held against the entry, because two questions
   * can stand at once — a session asks again the moment the first is answered —
   * and because the transcript rebuilds its entries as events arrive. Nothing is
   * cleaned up when one is answered: the id never comes back, and a handful of
   * dead keys costs less than the code to notice.
   */
  private readonly chosen = signal<Record<string, Answers>>({});
  /** What has been typed against a question instead of choosing, by the same key. */
  private readonly said = signal<Record<string, string>>({});
  /** Notes written beside a choice, by ask id and then question. Unlike [said]
   *  these travel *with* the choices — see `questions.ts`. */
  private readonly noted = signal<Record<string, Notes>>({});
  /** Which questions have their note field open, as `<ask>::<question>`. A card
   *  with a field under every question is a screenful before it says anything,
   *  so the field is one tap away rather than always there. */
  private readonly noting = signal<ReadonlySet<string>>(new Set());
  /** Whether anything older than what is on screen remains on disk.
   *
   *  The cursor is a byte offset into the transcript, so zero is the start of the
   *  file and this is simply "not at the beginning". */
  readonly more = computed(() => (this.held()?.cursor() ?? 0) > 0);
  /**
   * Whether this view is showing somewhere the reader jumped to, rather than the
   * live end of the conversation.
   *
   * ⚠ **Worth saying on screen, loudly.** Detached, the page does not grow and
   * the session's own state is unknown — so a reader who did not notice would
   * watch a working session say nothing and conclude it had stopped. See
   * [[SessionStore.goTo]].
   */
  readonly adrift = computed(() => this.held()?.adrift() ?? false);
  readonly loading = signal(false);
  private scroller = viewChild<ElementRef<HTMLElement>>('scroller');
  /** The top of what has been read, and the thing that asks for more. */
  private brink = viewChild<ElementRef<HTMLElement>>('brink');
  /**
   * How many pages have been fetched, only ever read to re-arm the observer.
   *
   * ⚠ **An IntersectionObserver reports transitions, not states.** After a page
   * lands, the mark is normally pushed out of view and the next crossing is a
   * real one — but when the page that arrived is shorter than the screen the
   * mark never leaves, no transition happens, and the reader is left at the top
   * of a conversation that has more and will not fetch it until they scroll.
   * Re-observing delivers a fresh initial callback, so bumping this after each
   * page is what makes "as much as the reader wants" true rather than
   * "as much as fits in one screenful more".
   */
  private pages = signal(0);

  constructor() {
    effect((onCleanup) => {
      const id = this.id();
      // Before anything is fetched: the toolbar needs to know which screen it is
      // drawing, and that is the route's answer rather than the runner's. See
      // [[Here.at]] for the flash this removes.
      this.here.at.set(id);
      this.held.set(this.store.open(id));
      // Covers leaving this session for another one and leaving it for the list:
      // the effect is cleaned up before it re-runs and again when the view goes.
      // What is left behind is the transcript, which is the point.
      onCleanup(() => this.store.leave(id));
      this.refresh();
      this.poll ??= setInterval(() => this.refresh(), 5000);
    });
    // A message being written belongs to the conversation, not to this view of
    // it — see [[Drafts]]. Two effects rather than one because they run in
    // opposite directions: this one puts a held draft into the composer when the
    // session opens, and it must not be reading the signals it writes.
    effect(() => {
      const id = this.id();
      untracked(() => {
        this.text.set(this.drafts.text(id));
        this.picture.set(this.drafts.picture(id));
      });
    });
    // And this one records every change back, keystroke by keystroke. It is also
    // how a draft is FORGOTTEN: a successful send empties the composer, which
    // arrives here as a draft with nothing in it.
    effect(() => {
      const id = this.id();
      const text = this.text();
      const picture = this.picture();
      untracked(() => this.drafts.put(id, text, picture));
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
    // ⚠ **A jump lands at the BOTTOM of the page it fetched, unconditionally** —
    // where [follow] below moves only for a reader already at the end. The
    // landmark is the last thing on that page (the cursor is the end of its
    // line, see `past::Landmark::at`), so the bottom is the thing that was
    // tapped. Following's politeness is exactly wrong here: somebody who asks to
    // be taken somewhere has said where they want to be.
    effect(() => {
      if (!this.held()?.adrift()) return;
      this.entries();
      requestAnimationFrame(() => {
        const box = this.scroller()?.nativeElement;
        if (!box) return;
        box.scrollTop = box.scrollHeight;
        // Told to following as a landing of ours, or the scroll event this
        // causes reads as the reader moving away and holds every later follow.
        this.following.landed(box.scrollTop);
      });
    });
    // A frame after the entries change, not with them: `follow` reads a height
    // that does not exist until the browser has laid the new nodes out.
    // `afterRenderEffect` was the first thing tried here and never ran — proven
    // by the layout harness, which kept reporting scrollY 0.
    effect(() => {
      this.entries();
      requestAnimationFrame(() => this.follow());
    });
    // The second hand, wound only while something is running — see [now]. Both
    // conditions matter: a session can be working with no call in flight (it is
    // writing), and a call can be running with the session reported idle (a
    // background task outlives the turn that started it).
    effect((onCleanup) => {
      const ticking =
        this.held()?.since() !== undefined ||
        this.entries().some(
          (entry) => entry.kind === 'tool' && entry.ok === undefined && !entry.unrecorded,
        );
      if (!ticking) return;
      const tick = setInterval(() => this.now.set(Date.now()), 1000);
      onCleanup(() => clearInterval(tick));
    });
    // ⚠ **The end of the transcript moves when the transcript does not.** The
    // composer sits above it as a fixed-size row, so every line typed takes a
    // line off the scrolling region: nobody scrolled, nothing arrived, and the
    // message being answered slides out of sight — measured at 65px for four
    // lines, and it goes further as the box grows. It reads as the page
    // randomly stopping, because the next event snaps it back.
    //
    // Watching the box itself rather than the composer covers every way it can
    // happen at once — the keyboard, a rotation, a growing composer — and asks
    // the same question each time: is the reader still meant to be at the end.
    //
    // ⚠ In an effect, not inline: `viewChild` is a signal that holds nothing
    // during construction, so wiring this up in the constructor body observes
    // `undefined` and silently never fires. That is what the first version of
    // this did, and the measurement below still read 65px with it in place.
    effect((onCleanup) => {
      const box = this.scroller()?.nativeElement;
      if (!box || typeof ResizeObserver === 'undefined') return;
      const resized = new ResizeObserver(() => this.follow());
      resized.observe(box);
      onCleanup(() => resized.disconnect());
    });
    // Reaching the top of what has been read is the request for what came
    // before it. Watched rather than handled in `onScroll`: that handler already
    // decides one thing from a position it cannot fully trust — see its own
    // warning — and a second question answered from the same measurement would
    // inherit the same race. An observer is told about the element instead.
    //
    // 400px of margin, so the page is asked for while the reader is still
    // reading rather than after they have run out. Re-armed per page: see
    // [pages].
    effect((onCleanup) => {
      this.pages();
      const mark = this.brink()?.nativeElement;
      const box = this.scroller()?.nativeElement;
      if (!mark || !box || typeof IntersectionObserver === 'undefined') return;
      const watch = new IntersectionObserver(
        (seen) => {
          if (seen.some((one) => one.isIntersecting)) this.loadEarlier();
        },
        { root: box, rootMargin: '400px 0px 0px 0px' },
      );
      watch.observe(mark);
      onCleanup(() => watch.disconnect());
    });
  }

  ngOnDestroy(): void {
    if (this.poll) clearInterval(this.poll);
    // Leaving the page leaves the conversation, so the toolbar stops claiming
    // to be inside one — otherwise the list of sessions is titled with whichever
    // one was open last.
    this.here.open.set(undefined);
    this.here.at.set(undefined);
    this.here.gist.set(undefined);
    this.here.tasks.set(undefined);
  }

  /** The header facts — cost, turns, whether it is working — come from the
   *  summary rather than the stream, because they are totals and a client that
   *  reconnected mid-session has not seen every event that built them. */
  private refresh(): void {
    // ⚠ **Tied to this view's life, and it has to be.** A request does not stop
    // when the page that made it does: leaving a session clears the open
    // conversation in `ngOnDestroy`, and a poll already in flight then lands and
    // puts it straight back — so the LIST was titled with the session just left,
    // with a ⋮ beside it whose menu acted on that session. The clear was already
    // here and was already right; what it could not do was outlast a reply.
    this.api
      .state()
      .pipe(takeUntilDestroyed(this.until))
      .subscribe({
        next: (state) => {
          const mine = state.sessions.find((s) => s.id === this.id());
          this.session.set(mine);
          // The toolbar sits above the router and cannot see the route, so the
          // page that knows which conversation this is has to say so — and both
          // the menu and the details sheet act on what is set here. The whole
          // summary, because the sheet shows nearly all of it.
          this.here.open.set(mine);
          // And what it is about, which the sheet shows in full where the card
          // has room for two lines. Keyed by conversation — see [[Here.gist]].
          this.here.gist.set(state.gists?.[this.id()]);
          // And how much of its own list is left, for the ⋮ menu's label. Same
          // keying, same reason — see [[Here.tasks]].
          this.here.tasks.set(state.tasks?.sessions?.[this.id()]);
          this.updates.saw(state.bundle);
          this.unreachable.set(this.reach.answered());
        },
        // Only once it has outlived a poll — see [[Reach]] for the measurement.
        error: (err: unknown) =>
          this.unreachable.set(this.reach.failed(`cannot reach the runner: ${reason(err)}`)),
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
    const to = this.following.wants(measure(box));
    if (to !== undefined) {
      box.scrollTop = to;
      // Where it actually landed, so the scroll event this causes can be told
      // apart from one the reader caused. See [[Following.moved]].
      this.following.landed(box.scrollTop);
      return;
    }
    // Asked to follow and declined. Which of the two reasons it was matters:
    // a hold ends by itself and a reader who has scrolled away does not, so a
    // page that never catches up is a different fault from one that follows
    // something nobody is watching.
    // ⚠ **`top` and `height` as well as the gap they make**, because a gap that
    // grew says nothing on its own: a reader dragging away and a session writing
    // raise it identically. Diagnosing the hold defect (#116) stalled exactly
    // here — the seconds after an unpin could not be attributed without asking
    // Pippijn what his hand had been doing.
    this.telemetry.measured(
      this.following.held ? 'holding' : 'stayed',
      `gap=${Math.round(box.scrollHeight - box.scrollTop - box.clientHeight)} top=${Math.round(box.scrollTop)} height=${box.scrollHeight} entries=${this.entries().length}`,
    );
  }

  /** A finger went down on the transcript, which suspends following until it
   *  comes off again — see [[Following.took]]. */
  protected took(): void {
    const box = this.scroller()?.nativeElement;
    if (!box) return;
    this.following.took(measure(box));
  }

  /** And came off. Catching up here rather than waiting for the next event: a
   *  session that has just finished writing sends nothing more, so a transcript
   *  released at that moment would sit one message short until the next turn. */
  protected released(): void {
    const box = this.scroller()?.nativeElement;
    if (!box) return;
    this.following.released(measure(box));
    this.follow();
  }


  /**
   * The view moved; the engine decides what it meant. See [[Following.moved]],
   * which holds every rule and the measurement behind it.
   */
  onScroll(): void {
    const box = this.scroller()?.nativeElement;
    if (!box) return;
    const followed = this.following.pinned;
    // ⚠ **Read before the engine is told**, because both change: `lastWrite` is
    // overwritten by the next write and `held` by the next touch, and an unpin
    // explained by the state *after* it is explained by the wrong state.
    const wrote = this.following.lastWrite;
    const held = this.following.held;
    const was = this.was;
    this.was = box.scrollTop;
    this.following.moved(measure(box));
    // ⚠ **The moment following stops, with the numbers that stopped it.**
    // Reported from a phone as a conversation opening part-way up and coming
    // right on a second open — which the layout harness cannot reproduce,
    // because it hands the seed over in one chunk and the real runner streams
    // it. Whether this was the reader's decision or a scroll nobody made is
    // exactly what the log has to settle, so it carries the position and how far
    // from the end that leaves us.
    if (followed && !this.following.pinned) {
      this.telemetry.measured(
        'unpinned',
        `gap=${Math.round(box.scrollHeight - box.scrollTop - box.clientHeight)} top=${Math.round(box.scrollTop)} was=${Math.round(was)} wrote=${Math.round(wrote)} held=${held} height=${box.scrollHeight} view=${box.clientHeight} entries=${this.entries().length} settled=${this.following.settled}`,
      );
    }
  }

  /** Where the reader is meant to be, and everything that decides it. */
  private readonly following = new Following();

  /**
   * Where the view was at the previous scroll event — for the trace alone.
   *
   * Direction is the one thing a single measurement cannot carry, and it is
   * exactly what separates a reader scrolling back from a box changing shape
   * underneath one who has not moved.
   */
  private was = 0;

  /**
   * The page before the one on screen.
   *
   * The transcript belongs to the store; what belongs here is the reader's
   * place, because only the view can measure it. On demand rather than eager:
   * the runner re-reads and re-parses the file to answer.
   *
   * Called by the observer above rather than by a control. A failed fetch —
   * the phone off the VPN, the Mac asleep — sets `trouble` and stops there:
   * nothing retries until the reader moves, which is what keeps an unreachable
   * console from becoming a request every frame.
   */
  /**
   * Come back to the live end of the conversation from a jump.
   *
   * Through the store, which throws the jumped-to page away before re-opening —
   * see [[SessionStore.rejoin]] for why keeping both would draw the same
   * conversation twice with a hole in it.
   */
  protected rejoin(): void {
    this.held.set(this.store.rejoin(this.id()));
  }

  private loadEarlier(): void {
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
            // Last, and after the position is restored: re-arming while the
            // mark is still where it was would ask for the next page from the
            // old geometry.
            this.pages.update((n) => n + 1);
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
    const picture = this.picture();
    // A picture is a whole message on its own — a screenshot with nothing said
    // is the commonest thing this carries — so either half is enough to send.
    if ((!text && !picture) || this.sending()) return;
    // Saying something is asking to see the answer. See [[Following.spoke]] —
    // this also settles the relayout that detached the transcript 110ms after a
    // tap on send, which no measurement could have told from a reader scrolling.
    this.following.spoke();
    this.sending.set(true);
    const sent = picture
      ? this.api.show(this.id(), picture.data, picture.mediaType, text)
      : this.api.send(this.id(), text);
    sent.subscribe({
      next: (summary) => {
        this.sending.set(false);
        this.trouble.set('');
        this.text.set('');
        this.drop();
        this.session.set(summary);
      },
      error: (err: unknown) => {
        this.sending.set(false);
        // ⚠ **The picture is kept on a failure.** It cost a scale and an upload
        // over a phone connection, and the commonest failure here is a runner
        // that was not reachable for a moment — losing it would mean choosing it
        // again from a gallery.
        this.trouble.set(reason(err));
      },
    });
  }

  /**
   * How long this session has been failing to read, in words.
   *
   * Coarse on purpose: the number is read to decide whether to restart, and
   * `21m` and `21m 14s` lead to the same decision.
   */
  readonly silence = computed(() => {
    const seconds = this.session()?.deaf ?? 0;
    if (seconds < 60) return `${seconds}s`;
    const minutes = Math.round(seconds / 60);
    return minutes < 60 ? `${minutes}m` : `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
  });

  /**
   * Stop a session that has stopped listening and start it again on the same
   * conversation.
   *
   * ⚠ **The only known cure, and it keeps the conversation.** The id, the
   * transcript and everything said survive; what is lost is the process's own
   * state and the messages still sitting in its pipe — which the runner hands
   * back, because that is the step somebody doing this by hand forgets. See
   * `roster::Roster::revive`.
   *
   * Tens of seconds, because the old process has to leave the process table
   * before the conversation may be resumed; [[reviving]] is what keeps the
   * button from being pressed twice in the meantime.
   */
  revive(id: string): void {
    if (this.reviving()) return;
    this.reviving.set(true);
    this.api.revive(id).subscribe({
      next: (summary) => {
        this.reviving.set(false);
        this.trouble.set('');
        this.session.set(summary);
      },
      error: (err: unknown) => {
        this.reviving.set(false);
        this.trouble.set(reason(err));
        this.telemetry.note('revive-refused', reason(err));
      },
    });
  }

  /**
   * Take what was chosen from the picker, scaled to something worth sending.
   *
   * ⚠ **The input is cleared afterwards, and it matters.** A file input holds
   * its selection, so choosing the same screenshot twice in a row fires no
   * `change` event the second time and the picker simply appears to do nothing.
   */
  chose(input: HTMLInputElement): void {
    const file = input.files?.[0];
    input.value = '';
    if (!file) return;
    this.pictureTrouble.set('');
    shrink(file)
      .then((picture) => {
        this.drop();
        this.picture.set(picture);
        this.telemetry.measured('picture', `${picture.width}x${picture.height} ${picture.bytes}B`);
      })
      .catch((err: unknown) => {
        this.pictureTrouble.set(`that image could not be read: ${reason(err)}`);
        this.telemetry.note('picture-refused', reason(err));
      });
  }

  /**
   * What this entry should be drawn as, which is not always what it is.
   *
   * ⚠ **One action, one widget** (memview#86). The CLI announces a call and then
   * asks whether it may run it — two events for one action — so the transcript
   * now hangs the question on the call's own entry rather than adding a card
   * beside it. A tool row with a question still on it draws as the question; the
   * moment it is answered it is an ordinary row again, and folds into the run it
   * belongs to.
   */
  drawn(entry: Entry): string {
    return entry.ask !== undefined && entry.allowed === undefined ? 'ask' : entry.kind;
  }

  /** Which held command is being taken back, so its × cannot be tapped twice. */
  readonly unholding = signal<string | undefined>(undefined);

  /**
   * Take back a command that is waiting for the turn to end.
   *
   * The answer is the session as it now is, so the chip goes when the runner
   * says it has gone rather than when the tap happens — the same rule as
   * everywhere else here: the screen reports what the runner did, not what this
   * page asked for.
   */
  unhold(command: string): void {
    this.unholding.set(command);
    this.api.unhold(this.id(), command).subscribe({
      next: (summary) => {
        this.unholding.set(undefined);
        this.session.set(summary);
      },
      error: (err: unknown) => {
        this.unholding.set(undefined);
        this.trouble.set(reason(err));
      },
    });
  }

  /** Put the held picture down, releasing what the preview holds open. */
  drop(): void {
    const held = this.picture();
    if (held) URL.revokeObjectURL(held.preview);
    this.picture.set(undefined);
    this.pictureTrouble.set('');
  }

  protected readonly weight = weight;

  /**
   * Where a picture in the transcript is fetched from.
   *
   * The entry carries a file name and the session id is this view's own input,
   * so nothing about a picture needs to travel through the fold.
   */
  pictureAt(entry: Entry): string {
    return this.api.pictureAt(this.id(), entry.picture ?? '');
  }

  /**
   * The picture currently shown at full width, if any.
   *
   * ⚠ **One at a time, and by name rather than by entry.** A conversation can
   * hold the same picture twice — a screenshot sent, discussed, and sent again —
   * and both should open together rather than one of them silently doing
   * nothing. Held here rather than on the entry for the same reason
   * [`opened`](#opened) is: it is about this reading, not about the conversation.
   */
  protected readonly full = signal<string | undefined>(undefined);

  /** Open a picture to the width of the column, or put it back. Thumbnails are
   *  the default because a transcript of full-width screenshots is a transcript
   *  you cannot scroll past. */
  enlarge(entry: Entry): void {
    this.full.update((open) => (open === entry.picture ? undefined : entry.picture));
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

  /**
   * Whether this row is a shell command there is a parse to show.
   *
   * ⚠ **Only `Bash`.** The reader reads shell, and the Python and the nested and
   * remote shells inside it — all of which arrive as a `Bash` call. Offering the
   * parse on an `Edit` or a `Read` would open a sheet that could only say the
   * text is not a command.
   */
  protected parseable(entry: Entry): boolean {
    return entry.tool === 'Bash' && !!entry.text.trim();
  }

  /**
   * Open the command, as written and as the index reads it.
   *
   * The row itself stays a single ellipsised line: this is where the whole text
   * lives, and there is nowhere else on the phone that holds it. See
   * `parse-sheet.ts` for why the two halves are stacked rather than switched
   * between.
   */
  protected parse(entry: Entry): void {
    this.dismiss.onBack(
      this.sheet.open(ParseSheet, {
        data: { session: this.id(), command: entry.text, ok: entry.ok },
        panelClass: 'session-sheet',
      }),
    );
  }

  /**
   * The transcript as it is drawn: runs of tool calls folded into one row.
   *
   * ⚠ **Grouped here rather than in [[fold]]**, so a result still finds its call
   * by id in a flat list. See `transcript.ts`.
   */
  readonly blocks = computed<Block[]>(() => blocks(this.entries()));

  /** What a folded run says about itself. */
  protected counted(block: Block & { kind: 'tools' }): ReturnType<typeof ran> {
    return ran(block.entries);
  }

  /**
   * Whether a run is open. Closed until somebody opens it, and that is the whole
   * rule.
   *
   * ⚠ **Two versions of "open it for them" were tried and both were worse.**
   * The first read `running > 0` live, which flickers: a session making one call
   * at a time turns a pair into a run and opens it, its result empties the run
   * and folds it, the next call opens it again — a dozen sequential calls, a
   * dozen flips, reported from the phone as "it keeps flipping open and closed".
   * The second latched that condition, so a run this page had watched work
   * stayed open. That stopped the flicker and cost more than it saved: the page
   * was no longer a function of the conversation. The same session rendered at
   * different heights on two screens, a reload collapsed whatever you had
   * accumulated, and a long working session stacked up open runs until it was
   * nearly as tall as it had been before any of this.
   *
   * What the automatic open was for — not looking idle while it works — the
   * summary row already does, because it says `3 running` on its face. So the
   * cost of this rule is one tap on the one run you care about, and what it buys
   * is a page that looks the same to everyone, at every reload.
   */
  protected opensTools(block: Block & { kind: 'tools' }): boolean {
    return this.toolChoice()[block.key] ?? false;
  }

  protected toggleTools(block: Block & { kind: 'tools' }): void {
    const open = this.opensTools(block);
    this.toolChoice.update((choice) => ({ ...choice, [block.key]: !open }));
  }

  /** What the reader has said about each run, which beats the default above. */
  private readonly toolChoice = signal<Record<string, boolean>>({});

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

  /**
   * What was decided, in the words of the thing that was decided.
   *
   * A question is not a permission, and saying a question was *allowed* would
   * describe the mechanism rather than what happened — the person picked an
   * option, or declined to. What they picked is in the tool's own result a line
   * below, which is where it reads best.
   */
  verdict(entry: Entry): string {
    // ⚠ **Until the session acts on it, this is a claim about the pipe.**
    // `Answered` is pushed once the decision has been written and flushed, which
    // is not the same as the CLI having read it — and against a session that has
    // stopped reading, the old wording reported the answer as delivered and
    // accepted while the session stayed blocked on the same question. `health`
    // showed a green *answered* for thirty-one minutes (memview #122). See
    // [[Entry.settling]].
    if (entry.settling) return 'sent — not taken up yet';
    if (!entry.questions) return entry.allowed ? 'allowed' : 'refused';
    if (!entry.allowed) return 'skipped';
    return entry.reply?.response?.trim() ? 'replied' : 'answered';
  }

  /** What was picked, for the row that records it. Empty when there is nothing
   *  to say — a refusal, or any tool that is not a question. */
  choice(entry: Entry): string {
    return entry.allowed ? choiceOf(entry.reply) : '';
  }

  /** Whether this option is currently chosen — what the button shows as pressed. */
  picked(entry: Entry, question: Question, label: string): boolean {
    const chosen = this.chosen()[entry.ask ?? '']?.[question.question];
    return Array.isArray(chosen) ? chosen.includes(label) : chosen === label;
  }

  /**
   * Choose an option.
   *
   * **One question with one answer sends on the tap.** That is the shape almost
   * every question has, and on a phone the difference between one tap and two is
   * the difference between answering from the lock screen and putting it off.
   * Anything else — several questions, or one that takes several answers — has
   * no moment where the choice is obviously finished, so it waits for [answer].
   */
  pick(entry: Entry, question: Question, label: string): void {
    if (!entry.ask || entry.allowed !== undefined || this.replying(entry)) return;
    const questions = entry.questions ?? [];
    const single = questions.length === 1 && !question.multiSelect;
    if (single) {
      this.approveWith(entry, { [question.question]: label }, undefined, this.noted()[entry.ask]);
      return;
    }
    const ask = entry.ask;
    this.chosen.update((all) => {
      const here = { ...(all[ask] ?? {}) };
      if (question.multiSelect) {
        const had = here[question.question];
        const list = Array.isArray(had) ? had : [];
        here[question.question] = list.includes(label)
          ? list.filter((l) => l !== label)
          : [...list, label];
      } else {
        here[question.question] = label;
      }
      return { ...all, [ask]: here };
    });
  }

  /** Whether everything asked has been answered. The send button waits for it. */
  ready(entry: Entry): boolean {
    const ask = entry.ask ?? '';
    return complete(entry.questions ?? [], this.chosen()[ask] ?? {}, this.noted()[ask] ?? {});
  }

  /** The note written against one question, if any. */
  note(entry: Entry, question: Question): string {
    return this.noted()[entry.ask ?? '']?.[question.question] ?? '';
  }

  /** Whether this question's note field is open — see [noting]. */
  notable(entry: Entry, question: Question): boolean {
    return (
      this.noting().has(`${entry.ask ?? ''}::${question.question}`) ||
      this.note(entry, question) !== ''
    );
  }

  /** Open the note field for one question. It never closes on its own: a field
   *  that vanished while it held words would be taking them away. */
  addNote(entry: Entry, question: Question): void {
    const key = `${entry.ask ?? ''}::${question.question}`;
    this.noting.update((open) => new Set([...open, key]));
  }

  jot(entry: Entry, question: Question, text: string): void {
    const ask = entry.ask;
    if (!ask) return;
    this.noted.update((all) => ({
      ...all,
      [ask]: { ...(all[ask] ?? {}), [question.question]: text },
    }));
  }

  /** What has been typed against this question, if anything. */
  words(entry: Entry): string {
    return this.said()[entry.ask ?? ''] ?? '';
  }

  /**
   * Whether this card is answering in words rather than by choice.
   *
   * ⚠ **The two are alternatives, not companions.** The CLI's result builder
   * tests `response` before `answers` and reports only the one it finds, so
   * words sent alongside a set of taps would throw the taps away without saying
   * so. Typing therefore takes the card over: the options go quiet, and clearing
   * the field hands it back. Better to make the exclusivity visible than to let
   * somebody tap four options and have none of them arrive.
   */
  replying(entry: Entry): boolean {
    return this.words(entry).trim() !== '';
  }

  say(entry: Entry, text: string): void {
    const ask = entry.ask;
    if (!ask) return;
    this.said.update((all) => ({ ...all, [ask]: text }));
  }

  /** Whether the button that sends is worth showing at all. */
  needsSending(entry: Entry): boolean {
    const questions = entry.questions ?? [];
    return this.replying(entry) || questions.length > 1 || (questions[0]?.multiSelect ?? false);
  }

  /** Send what has been chosen, or what has been typed instead of choosing. */
  answer(entry: Entry): void {
    if (!entry.ask || entry.allowed !== undefined) return;
    if (this.replying(entry)) {
      // Words override the choices in the CLI, so nothing else goes with them —
      // notes included, which would be qualifying an answer that is not sent.
      this.approveWith(entry, undefined, this.words(entry).trim(), undefined);
      return;
    }
    if (!this.ready(entry)) return;
    this.approveWith(entry, this.chosen()[entry.ask] ?? {}, undefined, this.noted()[entry.ask]);
  }

  /** Approve the call with the answer written into it. See `questions.ts`. */
  private approveWith(entry: Entry, answers?: Answers, response?: string, notes?: Notes): void {
    if (!entry.ask || entry.allowed !== undefined) return;
    this.api.decide(this.id(), entry.ask, true, undefined, answers, response, notes).subscribe({
      error: (err: unknown) => this.trouble.set(reason(err)),
    });
  }
}
