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
import { toObservable, toSignal } from '@angular/core/rxjs-interop';
import { switchMap } from 'rxjs';
import { MatBottomSheet } from '@angular/material/bottom-sheet';
import { MatButtonModule } from '@angular/material/button';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { NgTemplateOutlet } from '@angular/common';

import { AskCard } from './ask-card';
import { Clock } from './clock';
import { Composer } from './composer';
import { Lasted } from './lasted';
import { ConsoleApi } from './console-api';
import { reason } from './errors';
import { Roster } from './roster';
import { Foreground } from './foreground';
import { Entry, Summary } from './models';
import { modelName } from './model';
import { modeIcon, modeIsLoud, modeTitle } from './modes';
import { Dismiss } from './dismiss';
import { Drafts, type Resolution } from './drafts';
import { since } from './since';
import { Folding } from './folding';
import { Following, measure } from './following';
import { Here } from './here';
import { Updates } from './updates';
import { Coloured } from './coloured';
import { PICTURE, Rendered } from './rendered';
import { pointedAt, shrink } from './picture';
import { Held, SessionStore } from './session-store';
import { ParseSheet } from './parse-sheet';
import { PictureSheet } from './picture-sheet';
import { Block, blocks, ran } from './transcript';
import { Telemetry } from './telemetry';
import { fullness } from './tokens';

/** One session: what it has done, and the way to say something to it. */
@Component({
  selector: 'app-session-view',
  templateUrl: './session-view.html',
  styleUrl: './session-view.scss',
  // ⚠ **On the host, not on the transcript element, and that is an
  // accessibility rule's doing rather than a design choice.** A `(click)` in the
  // template draws `click-events-have-key-events` and
  // `interactive-supports-focus`: an `<ol>` that handles taps is unreachable by
  // keyboard. Both are false here — what is tapped is an anchor, which is
  // focusable and turns Enter into this very event — but the rule reads
  // templates and cannot see what the handler is waiting for. Making the list
  // focusable to satisfy it would put a stop on the tab order that leads
  // nowhere. Pinned instead by the harness, which reaches the link with the
  // keyboard and opens it.
  host: { '(click)': 'tapped($event)' },
  imports: [
    AskCard,
    Clock,
    Composer,
    Lasted,
    NgTemplateOutlet,
    MatButtonModule,
    MatIconModule,
    MatProgressBarModule,
    Coloured,
    Rendered,
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
  private roster = inject(Roster);

  /**
   * What the COLLECTION holds for the conversation on screen, text and picture.
   *
   * ⚠ **Keyed on the route, so switching conversations switches the stream.**
   * `undefined` means the collection has not answered yet — not that the draft
   * is empty — and the effects below wait for that rather than blanking the box.
   */
  private readonly stored = toSignal(
    toObservable(this.id).pipe(switchMap((id) => this.drafts.text$(id))),
    { initialValue: undefined },
  );
  private readonly storedPicture = toSignal(
    toObservable(this.id).pipe(switchMap((id) => this.drafts.picture$(id))),
    { initialValue: undefined },
  );
  private telemetry = inject(Telemetry);
  private foreground = inject(Foreground);
  private until = inject(DestroyRef);
  /** For `afterNextRender` outside an injection context — see [loadEarlier]. */
  private injector = inject(Injector);

  /** The transcript being read, which outlives this view — see [[SessionStore]]. */
  /** What this reading of the conversation has opened — per view, see
   *  [[Folding]]. */
  protected readonly folding = new Folding();
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
  readonly unreachable = this.roster.unreachable;
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
  /**
   * A picture chosen and scaled, waiting to go with the next message.
   *
   * ⚠ **The STORED one, not a second copy.** Holding it here as well meant two
   * signals describing one picture, and they are never identical objects — the
   * chosen one carries a `blob:` preview and the stored one a data URL — so each
   * update of one retriggered the other. Same rule as the words: one source.
   */
  readonly picture = this.storedPicture;
  /** What went wrong choosing one — too large, not an image, a phone that
   *  refused. On the composer rather than in the transcript: it is about the
   *  thing being written, not about the conversation. */
  readonly pictureTrouble = signal('');
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
  /** Whether this is a copy kept on the phone rather than the conversation. See
   *  [[Kept]] and the banner it draws. */
  readonly stale = computed(() => this.held()?.stale() ?? false);
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
  /**
   * Whether the view is where `follow` means it to be, or a reposition is
   * still pending.
   *
   * ⚠ **The observer below must not be armed while this is false.** An
   * `IntersectionObserver` delivers a guaranteed initial callback with the
   * CURRENT state, and `follow` repositions a FRAME later — so on a busy main
   * thread the callback arrived while `scrollTop` was still 0, the brink mark
   * sat inside the 400px margin, and a page was fetched for a reader sitting
   * at the newest message. Reproduced 2026-09-11 by replacing that frame with
   * a 150ms timeout (memview#1243).
   *
   * ⚠ **Cleared on EVERY entries change, not once at startup.** The transcript
   * arrives progressively, so `follow` re-scrolls after each change and the
   * window reopens each time. A first-placement-only guard was tried and
   * REFUTED — it passed unperturbed and still failed under the perturbation.
   */
  private settled = signal(false);

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
    });
    // What the runner is holding, polled once for the app — see [[Roster]].
    // Followed rather than fetched: a request does not stop when the page that
    // made it does, and a reply landing after this view is gone used to put the
    // session just left back into the toolbar.
    this.until.onDestroy(this.roster.follow());
    // Derived, so leaving cannot be undone by a reply in flight.
    effect(() => {
      const state = this.roster.state();
      const id = this.id();
      untracked(() => {
        const mine = state?.sessions.find((s) => s.id === id);
        this.session.set(mine);
        // The toolbar sits above the router and cannot see the route, so the
        // page that knows which conversation this is has to say so — and both
        // the menu and the details sheet act on what is set here.
        this.here.open.set(mine);
        // And what it is about, which the sheet shows in full where the card has
        // room for two lines. Keyed by conversation — see [[Here.gist]].
        this.here.gist.set(state?.gists?.[id]);
        // And how much of its own list is left, for the ⋮ menu's label.
        this.here.tasks.set(state?.tasks?.sessions?.[id]);
      });
    });
    // A message being written belongs to the conversation, not to this view of
    // it — see [[Drafts]], which holds ONE copy of it, in the collection.
    //
    // ⚠ **Two effects, and they cannot fight.** The first renders the document,
    // the second records what is typed. They would loop if a write could come
    // back as a different value — but a write goes straight to the document, so
    // what arrives back is what was sent, and the guard clause below stops
    // there. Every "is this me or the other device" check that used to live here
    // and in `Drafts` existed because a debounce put a gap between the two.
    effect(() => {
      const held = this.stored();
      untracked(() => {
        // ⚠ Undefined means the collection has not answered yet, which is NOT
        // the same as an empty draft — setting the box to `''` on that would
        // blank what somebody is typing while the database opens.
        if (held === undefined || held === this.text()) return;
        this.text.set(held);
      });
    });
    effect(() => {
      const id = this.id();
      const text = this.text();
      untracked(() => {
        // Nothing is written before the collection has answered: until then this
        // box is empty because nothing has been read, not because nothing is
        // there, and recording that would erase the draft.
        if (this.stored() === undefined) return;
        void this.drafts.write(id, text);
      });
    });
    // Ask the runner on opening rather than waiting out the heartbeat — see
    // [[ConsoleDb.resync]].
    effect(() => {
      this.id();
      untracked(() => this.drafts.sync());
    });
    // The poll does not run while the phone is away, so the header facts on
    // screen when it comes back are as old as the pocket it was in. The
    // transcript below them heals itself — EventSource reconnects and replays
    // from the top — and these totals have nothing that would.
    this.foreground.onReturn(() => this.roster.ask(), this.until);
    // And the draft, for the same reason the poll pairs with this: the other
    // device may have carried it on, and anything typed here while the tunnel
    // was down is still owed — see [[ConsoleDb.resync]].
    this.foreground.onReturn(() => this.drafts.sync(), this.until);
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
      // A reposition is now owed; the observer stays disarmed until it lands.
      this.settled.set(false);
      requestAnimationFrame(() => {
        // ⚠ **`finally`, because this signal being false DISABLES the observer**
        // — so anything that skips the set kills earlier-loading for the rest of
        // the session, not merely for this frame. `follow` reads layout,
        // consults a state machine and writes telemetry; a throw anywhere in
        // there left `settled` false for ever, and a reader who then scrolled to
        // the top got nothing, silently and permanently.
        //
        // ⚠ **The hole did not exist before 9ae8a82** and is that commit's
        // doing: until then a fault in `follow` cost one reposition, because
        // nothing else waited on it having finished. Measured 2026-09-12 by
        // injecting a throw after `follow`'s real work — 5/5 runs of the
        // earlier-fetch test failed on the shipped code, 5/5 passed both with
        // this `finally` and on the pre-9ae8a82 code, which is the asymmetry
        // that names the cause. Same assertion the nightly failed at 03:40 that
        // morning (memview#1243).
        try {
          this.follow();
        } finally {
          // Set even when `follow` DECLINES — a reader who has scrolled away is
          // settled where they put themselves.
          this.settled.set(true);
        }
      });
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
      // ⚠ **Disarmed while a reposition is owed — see [settled].** Reading the
      // signal here means the effect re-runs when it flips, so the observer is
      // disconnected by the cleanup and re-observed after the scroll lands,
      // which delivers a FRESH initial callback against the settled position.
      // Merely ignoring the callback would not: an observer reports
      // transitions, and a dropped one does not come back.
      if (!this.settled()) return;
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

  /** How long ago, from a millisecond timestamp. See [[since]]. */
  ago(at: number): string {
    return since(at);
  }

  /** Both texts, when this device and the other have each written one. */
  readonly clash = computed(() => {
    const clash = this.drafts.clash();
    return clash?.id === this.id() ? clash : undefined;
  });

  /** Settle it, and put the settled text straight into the composer. */
  settle(how: Resolution): void {
    const clash = this.clash();
    if (!clash) return;
    // The box follows the document, so nothing is set here: `resolve` writes the
    // settled text and the effect above renders it.
    void this.drafts.resolve(clash.id, clash.theirs, how);
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

  /** Take what was chosen from the picker, scaled to something worth sending. */
  chose(file: File): void {
    this.pictureTrouble.set('');
    shrink(file)
      .then((picture) => {
        // The blob URL dies with this document; what is stored carries the
        // bytes and a data URL is built on the way out.
        URL.revokeObjectURL(picture.preview);
        void this.drafts.hold(this.id(), picture);
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

  /** Put the held picture down. */
  drop(): void {
    void this.drafts.hold(this.id(), undefined);
    this.pictureTrouble.set('');
  }

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
   * Open a link to a picture here, rather than letting it leave the app.
   *
   * ⚠ **What this replaces did something, and the something was wrong.** GFM
   * autolinks a bare URL, so these were already anchors; the shell hands any
   * host outside its own to the phone's browser, and the addresses a session
   * writes name this Mac's LAN, which the phone is not on. So a tap left the
   * console and arrived nowhere. See `picture.ts` for how the link is marked and
   * `console::images::fetch` for who does the fetching.
   *
   * Nothing is prevented until a picture link is found: this listener sits on
   * the whole transcript, and every other tap in it belongs to something else.
   */
  protected tapped(event: MouseEvent): void {
    if (!(event.target instanceof Element)) return;
    const link = event.target.closest(`a.${PICTURE}`);
    const url = link && pointedAt(link.getAttribute('href') ?? '');
    if (!url) return;
    event.preventDefault();
    this.dismiss.onBack(
      this.sheet.open(PictureSheet, { data: { url }, panelClass: 'picture-panel' }),
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
}
