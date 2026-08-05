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
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { TextFieldModule } from '@angular/cdk/text-field';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { NgTemplateOutlet } from '@angular/common';

import { Clock } from './clock';
import { ConsoleApi } from './console-api';
import { reason } from './errors';
import { Foreground } from './foreground';
import { Entry, Summary } from './models';
import { modelName } from './model';
import { modeIcon, modeIsLoud, modeTitle } from './modes';
import { Here } from './here';
import { Updates } from './updates';
import { Rendered } from './rendered';
import { Answers, Notes, Question, choiceOf, complete } from './questions';
import { Held, SessionStore } from './session-store';
import { Block, blocks, ran } from './transcript';
import { fullness } from './tokens';

/** One session: what it has done, and the way to say something to it. */
@Component({
  selector: 'app-session-view',
  templateUrl: './session-view.html',
  styleUrl: './session-view.scss',
  imports: [
    Clock,
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
   * Shown so compaction can be seen coming rather than met. Formatted where the
   * list formats the same fact — see [[fullness]], and the row in
   * `sessions-view` that reads it for a conversation that is not running.
   */
  readonly context = computed(() => fullness(this.session()?.context, this.session()?.window));

  /** How many background tasks the harness has told us about and not closed. */
  readonly background = computed(() => this.held()?.background().length ?? 0);
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
  readonly sending = signal(false);
  readonly text = signal('');
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
      // Where we put it, so the scroll event this causes can be told apart from
      // one the reader caused. See [onScroll].
      this.wrote = box.scrollTop;
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

  /**
   * `NEAR` is the slack: a few lines, so a partly-scrolled view still counts as
   * following rather than as having been left behind.
   *
   * ⚠ **A scroll this component performed is not a reader's decision**, and
   * failing to tell the two apart is what made following stop at random. The
   * sequence, measured: `follow` sets `scrollTop` to the bottom and the browser
   * queues a scroll event; more of the answer renders before that event is
   * delivered; the handler then runs against the NEW `scrollHeight` and the OLD
   * `scrollTop`, computes a gap of one or two deltas' worth — 120px to 168px,
   * where the slack is 120 — and files the reader as having scrolled away. From
   * then on nothing follows, and nobody touched the screen. It bit two runs in
   * five of the deltas measurement.
   *
   * Remembering where we put it is the whole fix: a scroll that lands exactly
   * there is ours, and says nothing about where the reader wants to be.
   */
  onScroll(): void {
    const box = this.scroller()?.nativeElement;
    if (!box) return;
    if (box.scrollTop === this.wrote) return;
    const NEAR = 120;
    this.pinned = box.scrollHeight - box.scrollTop - box.clientHeight < NEAR;
    this.wrote = -1;
  }

  /** The last scroll position this component set, or -1 for none outstanding. */
  private wrote = -1;

  /** Whether the first render has happened; before it there is nothing to keep. */
  private settled = false;

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
