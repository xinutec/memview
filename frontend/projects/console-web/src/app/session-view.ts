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
import { MatButtonModule } from '@angular/material/button';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { Pictures, Sheets, scaffoldTitle } from '@xinutec/ui-scaffold';

import { Composer } from './composer';
import { ConsoleApi } from './console-api';
import { Drafts } from './drafts';
import { reason } from './errors';
import { Folding } from './folding';
import { Following, measure } from './following';
import { Foreground } from './foreground';
import { Here } from './here';
import { Lasted } from './lasted';
import { EffortBars } from './effort-bars';
import { LapseBanner } from './lapse-banner';
import { modelName } from './model';
import { titleOf } from './naming';
import {
  type Asked,
  type Stands,
  type Change,
  type Commands,
  type Entry,
  type Summary,
  type ToolCall,
} from './models';
import { modeIcon, modeIsLoud, modeTitle } from './modes';
import { Notice, NoticeBar, notice } from './notice';
import { DiffSheet } from './diff-sheet';
import { ParseSheet } from './parse-sheet';
import { PICTURE } from './rendered';
import { MOST, type Picture, looking, pointedAt, shrink } from './picture';
import { Outbox, type Standing, age, behindRefused, standing } from './outbox';
import { Roster } from './roster';
import { SignInSheet } from './signin-sheet';
import { Held, SessionStore } from './session-store';
import { Telemetry } from './telemetry';
import { fullness } from './tokens';
import { TranscriptList } from './transcript-list';
import { Block, blocks } from './transcript';
import { Updates } from './updates';

/** One conversation: its transcript, what it is doing, and the box to reply in. */
/** How long news, as against a fault, stays on the notice bar. */
const BRIEFLY_MS = 6_000;

/** How a message the Mac holds stands, for the line under it. */
function stands(how: Stands, deaf: boolean, waited: string): Standing {
  switch (how) {
    case 'held':
      return { line: 'runs when this turn ends', icon: 'hourglass_top' };
    case 'unread':
      return deaf
        ? {
            line: 'not read — the session has stopped listening',
            icon: 'hearing_disabled',
            alert: true,
          }
        : { line: `waiting to be read${waited}`, icon: 'schedule' };
    case 'dropped':
      return { line: 'not read — the session ended', icon: 'error', alert: true };
  }
}

/** How many pictures a waiting message carries, in words. */
function pictures(count: number): string {
  return `${count} picture${count === 1 ? '' : 's'}`;
}

@Component({
  selector: 'app-session-view',
  templateUrl: './session-view.html',
  styleUrl: './session-view.scss',
  host: { '(click)': 'tapped($event)' },
  imports: [
    Composer,
    EffortBars,
    LapseBanner,
    Lasted,
    MatButtonModule,
    MatIconModule,
    MatProgressBarModule,
    NoticeBar,
    TranscriptList,
  ],
})
export class SessionView implements OnDestroy {
  readonly id = input.required<string>();

  private readonly api = inject(ConsoleApi);
  private readonly sheets = inject(Sheets);
  private readonly outbox = inject(Outbox);
  private readonly pictures = inject(Pictures);
  private readonly here = inject(Here);
  private readonly store = inject(SessionStore);
  private readonly drafts = inject(Drafts);
  private readonly roster = inject(Roster);
  private readonly telemetry = inject(Telemetry);
  private readonly foreground = inject(Foreground);
  private readonly until = inject(DestroyRef);
  private readonly injector = inject(Injector);

  readonly updateWaiting = inject(Updates).waiting;

  // What the store holds for this conversation, and what is derived from it.
  private readonly held = signal<Held | undefined>(undefined);
  readonly session = signal<Summary | undefined>(undefined);
  /**
   * The conversation, then what the session has not read yet: this phone's
   * messages from the outbox, and commands held for the turn's end that this
   * phone did not send. One place for every message, whatever stands between it
   * and being read — see [[Outbox]].
   */
  readonly entries = computed<Entry[]>(() => {
    const mine = this.outbox.all().filter((it) => it.session === this.id());
    const now = this.minute();
    const session = this.session();
    // Read since the summary was drawn: the conversation has it now.
    const read = new Set(
      this.conversation().flatMap((entry) =>
        entry.kind === 'asked' && entry.id !== undefined ? [entry.id] : [],
      ),
    );
    const theirs = (session?.messages ?? [])
      .filter((message) => !read.has(message.id))
      .map((message): Asked => ({
        kind: 'asked',
        text: message.pictures?.length
          ? `${message.text}${message.text ? ' · ' : ''}${pictures(message.pictures.length)}`
          : message.text,
        at: message.at,
        id: message.id,
        waiting: stands(message.stands, !!session?.deaf, age(now - message.at)),
      }));
    const sending = mine.map((it): Asked => ({
      kind: 'asked',
      text: it.pictures?.length
        ? `${it.text}${it.text ? ' · ' : ''}${pictures(it.pictures.length)}`
        : it.text,
      at: it.created,
      id: it.id,
      waiting: standing(it, now, behindRefused(mine, it)),
    }));
    const said = this.conversation();
    const taking = this.taking();
    return [...said, ...theirs, ...sending].map((entry) =>
      entry.kind === 'asked' && taking.has(entry.id ?? entry.text)
        ? { ...entry, taking: true }
        : entry,
    );
  });

  /** The conversation as the session's events tell it. */
  private readonly conversation = computed<Entry[]>(() => {
    const held = this.held();
    if (!held) return [];
    const edited = held.edited();
    const diverged = held.diverged();
    return held.entries().map((entry) => {
      if (entry.kind !== 'tool' || !entry.call) return entry;
      const hunks = edited.get(entry.call);
      if (!hunks) return entry;
      return { ...entry, hunks, diverged: diverged.has(entry.call) };
    });
  });
  readonly blocks = computed<Block[]>(() => blocks(this.entries()));
  // Not under a kept copy: the cursor is the arriving page's, not the copy's.
  readonly more = computed(
    () => (this.held()?.cursor() ?? 0) > 0 && this.held()?.source() !== 'kept',
  );
  readonly adrift = computed(() => this.held()?.adrift() ?? false);
  /** What is below came off this phone, and the Mac has failed to answer. See [[Kept]]. */
  readonly kept = computed(
    () => this.held()?.source() === 'kept' && (this.held()?.unanswered() ?? false),
  );
  /** What is below came off this phone, and the conversation is on its way. */
  readonly catchingUp = computed(
    () => this.held()?.source() === 'kept' && !(this.held()?.unanswered() ?? false),
  );
  /** The one thing worth saying about the link, of the three that might be. */
  readonly notice = computed<Notice | undefined>(() =>
    notice({
      acting: this.trouble(),
      runner: this.roster.notice(),
      stream: this.held()?.link.worth(),
    }),
  );
  readonly doing = computed(
    () => this.held()?.doing() ?? (this.held()?.spoken() ? undefined : this.session()?.busy),
  );
  readonly arriving = computed(() => (this.session()?.waiting ? undefined : this.doing()));
  readonly inTurn = computed(() => this.session()?.working ?? false);
  readonly state = computed(() => (this.inTurn() ? 'working' : 'idle'));
  readonly ended = computed(() => {
    const session = this.session();
    return !!session && !session.alive;
  });
  readonly context = computed(() => fullness(this.session()?.context, this.session()?.window));
  readonly background = computed(() => this.session()?.background ?? 0);
  readonly running = computed(() => this.session()?.running ?? []);
  readonly mode = computed(() => modeTitle(this.session()?.mode));
  readonly modeIcon = computed(() => modeIcon(this.session()?.mode));
  readonly loud = computed(() => modeIsLoud(this.session()?.mode));
  readonly model = computed(() => modelName(this.session()?.model));
  /** When the shared login lapses. See [[LapseBanner]]. */
  readonly lapses = computed(() => this.roster.state()?.lapses);
  readonly silence = computed(() => {
    const seconds = this.session()?.deaf ?? 0;
    if (seconds < 60) return `${seconds}s`;
    const minutes = Math.round(seconds / 60);
    return minutes < 60 ? `${minutes}m` : `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
  });

  // A clock, ticking only while something is running.
  readonly now = signal(Date.now());
  readonly working = computed(() => {
    const since = this.held()?.since();
    return since === undefined ? undefined : this.now() - since;
  });

  // The composer's state. The text is a draft, kept and synced by `Drafts`.
  readonly text = signal('');
  /** What this session can be told, asked for on the first `/`. See [[suggest]]. */
  readonly commands = signal<Commands | undefined>(undefined);
  private askingCommands = false;
  readonly trouble = signal('');
  /** For how long each waiting message has waited, redrawn each minute. */
  readonly minute = signal(Date.now());
  /** Messages being taken back, by id (or words, for a command held without one). */
  private readonly taking = signal<ReadonlySet<string>>(new Set());
  readonly pictureTrouble = signal('');
  private readonly stored = toSignal(
    toObservable(this.id).pipe(switchMap((id) => this.drafts.text$(id))),
    { initialValue: undefined },
  );
  readonly chosen = toSignal(
    toObservable(this.id).pipe(switchMap((id) => this.drafts.pictures$(id))),
    { initialValue: [] },
  );
  readonly reviving = signal(false);
  readonly loading = signal(false);
  protected readonly full = signal<string | undefined>(undefined);
  protected readonly folding = new Folding();
  protected readonly pictureAt = (name: string): string => this.api.pictureAt(this.id(), name);

  // Scrolling: follow the end unless the reader has scrolled away.
  private readonly list = viewChild(TranscriptList);
  private readonly scroller = computed(() => this.list()?.host);
  private readonly brink = viewChild<ElementRef<HTMLElement>>('brink');
  private readonly following = new Following();
  private readonly pages = signal(0);
  private readonly settled = signal(false);
  private was = 0;

  constructor() {
    // The Mac's answer to a send shows the message it just took, before the next poll.
    effect(() => {
      const answered = this.outbox.answered();
      if (answered?.id === this.id()) this.session.set(answered);
    });
    const ticking = setInterval(() => this.minute.set(Date.now()), 60_000);
    this.until.onDestroy(() => clearInterval(ticking));
    // Its name, else where it runs, quieter: the same rule titles the list's cards.
    // Nothing during the round trip, rather than a stand-in for a stand-in.
    scaffoldTitle(() => {
      const open = this.session();
      return open && { text: titleOf(open), provisional: !open.name };
    });
    effect((onCleanup) => {
      const id = this.id();
      this.here.at.set(id);
      this.held.set(this.store.open(id));
      onCleanup(() => this.store.leave(id));
    });
    this.until.onDestroy(this.roster.follow());
    effect(() => {
      const state = this.roster.state();
      const id = this.id();
      untracked(() => {
        const mine = state?.sessions.find((s) => s.id === id);
        this.session.set(mine);
        this.here.open.set(mine);
        this.here.gist.set(state?.gists[id]);
        this.here.tasks.set(state?.tasks.sessions[id]);
      });
    });

    // Another conversation can be told other things: its folder's skills.
    effect(() => {
      this.id();
      untracked(() => this.commands.set(undefined));
    });
    effect(() => {
      const id = this.id();
      if (!this.text().startsWith('/')) return;
      untracked(() => {
        if (this.commands() || this.askingCommands) return;
        this.askingCommands = true;
        // `null` until the session has answered: the next keystroke asks again.
        this.api.commands(id).subscribe({
          next: (commands) => {
            if (commands && id === this.id()) this.commands.set(commands);
          },
          error: () => (this.askingCommands = false),
          complete: () => (this.askingCommands = false),
        });
      });
    });

    // The draft seeds the box; the box writes the draft.
    effect(() => {
      const held = this.stored();
      untracked(() => {
        if (held === undefined || held === this.text()) return;
        this.text.set(held);
      });
    });
    effect(() => {
      const id = this.id();
      const text = this.text();
      untracked(() => {
        if (this.stored() === undefined) return;
        void this.drafts.write(id, text);
      });
    });
    effect(() => {
      this.id();
      untracked(() => this.drafts.sync());
    });
    this.foreground.onReturn(() => this.roster.ask(), this.until);
    this.foreground.onReturn(() => this.drafts.sync(), this.until);

    // The soft keyboard resizes the viewport; keep the end in view through it.
    const viewport = window.visualViewport;
    if (viewport) {
      const settle = () => requestAnimationFrame(() => this.follow());
      viewport.addEventListener('resize', settle);
      this.until.onDestroy(() => viewport.removeEventListener('resize', settle));
    }
    effect(() => {
      if (!this.held()?.adrift()) return;
      this.entries();
      requestAnimationFrame(() => {
        const box = this.scroller()?.nativeElement;
        if (!box) return;
        box.scrollTop = box.scrollHeight;
        this.following.landed(box.scrollTop);
      });
    });
    effect(() => {
      this.entries();
      this.settled.set(false);
      requestAnimationFrame(() => {
        try {
          this.follow();
        } finally {
          this.settled.set(true);
        }
      });
    });
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
    effect((onCleanup) => {
      const box = this.scroller()?.nativeElement;
      if (!box || typeof ResizeObserver === 'undefined') return;
      const resized = new ResizeObserver(() => this.follow());
      resized.observe(box);
      onCleanup(() => resized.disconnect());
    });
    // Scrolling to the top fetches the page before it, once the view has settled.
    effect((onCleanup) => {
      this.pages();
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
    this.here.open.set(undefined);
    this.here.at.set(undefined);
    this.here.gist.set(undefined);
    this.here.tasks.set(undefined);
  }

  private follow(): void {
    const box = this.scroller()?.nativeElement;
    if (!box) return;
    const to = this.following.wants(measure(box));
    if (to !== undefined) {
      box.scrollTop = to;
      this.following.landed(box.scrollTop);
      return;
    }
    this.telemetry.measured(
      this.following.held ? 'holding' : 'stayed',
      `gap=${Math.round(box.scrollHeight - box.scrollTop - box.clientHeight)} top=${Math.round(box.scrollTop)} height=${box.scrollHeight} entries=${this.entries().length}`,
    );
  }

  protected took(): void {
    const box = this.scroller()?.nativeElement;
    if (box) this.following.took(measure(box));
  }

  protected released(): void {
    const box = this.scroller()?.nativeElement;
    if (!box) return;
    this.following.released(measure(box));
    this.follow();
  }

  onScroll(): void {
    const box = this.scroller()?.nativeElement;
    if (!box) return;
    const followed = this.following.pinned;
    const wrote = this.following.lastWrite;
    const held = this.following.held;
    const was = this.was;
    this.was = box.scrollTop;
    this.following.moved(measure(box));
    if (followed && !this.following.pinned) {
      this.telemetry.measured(
        'unpinned',
        `gap=${Math.round(box.scrollHeight - box.scrollTop - box.clientHeight)} top=${Math.round(box.scrollTop)} was=${Math.round(was)} wrote=${Math.round(wrote)} held=${held} height=${box.scrollHeight} view=${box.clientHeight} entries=${this.entries().length} settled=${this.following.settled}`,
      );
    }
  }

  protected rejoin(): void {
    this.held.set(this.store.rejoin(this.id()));
  }

  private loadEarlier(): void {
    if (this.loading()) return;
    const box = this.scroller()?.nativeElement;
    const before = box?.scrollHeight ?? 0;
    this.loading.set(true);
    this.store.earlier(this.id()).subscribe({
      next: () => {
        this.loading.set(false);
        this.trouble.set('');
        afterNextRender(
          () => {
            if (box) box.scrollTop += box.scrollHeight - before;
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

  /**
   * Into the outbox, which sends it now or when the Mac can be reached, and keeps
   * trying under the same id until the Mac confirms it — see [[Outbox]].
   */
  send(): void {
    const text = this.text().trim();
    const pictures = this.chosen();
    if (!text && !pictures.length) return;
    this.following.spoke();
    void this.outbox.enqueue(this.id(), text, pictures);
    this.trouble.set('');
    this.text.set('');
    this.dropAll();
  }

  /**
   * A message the session has not read, back into the box: this phone's from the
   * outbox, one sent elsewhere or held for the turn's end from the Mac, which asks
   * the CLI. Refused, and said why, once the session has read it.
   */
  async takeBack(entry: Asked): Promise<void> {
    // One at a time per message: taps arrive faster than the Mac answers, and each
    // answer would put the words in the box again.
    const key = entry.id ?? entry.text;
    if (this.taking().has(key)) return;
    this.taking.update((taking) => new Set(taking).add(key));
    try {
      await this.takeOne(entry);
    } finally {
      this.taking.update((taking) => {
        const left = new Set(taking);
        left.delete(key);
        return left;
      });
    }
  }

  private async takeOne(entry: Asked): Promise<void> {
    if (entry.id === undefined) return;
    // The outbox puts the words in this conversation's draft, which the box shows.
    const taken = await this.outbox.takeBack(this.id(), entry.id);
    switch (taken.kind) {
      case 'back':
        if (taken.session?.id === this.id()) this.session.set(taken.session);
        this.trouble.set('');
        break;
      case 'refused':
        this.briefly(`Not taken back: ${taken.why}.`);
        break;
      case 'unreachable':
        this.briefly(
          'Not taken back: the Mac could not be reached, and it may already have the message.',
        );
        break;
    }
  }

  /**
   * Say something that is news rather than a fault, for long enough to read: a
   * take-back that came too late changes nothing that needs mending, so it does
   * not stay until the next send the way a failure does.
   */
  private briefly(what: string): void {
    this.trouble.set(what);
    setTimeout(() => {
      if (this.trouble() === what) this.trouble.set('');
    }, BRIEFLY_MS);
  }

  /** Sign the account in again. See [[SignInSheet]]. */
  signIn(): void {
    this.sheets.open(SignInSheet, { panelClass: 'start-sheet' });
  }

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

  chose(files: readonly File[]): void {
    void this.add(files);
  }

  /** Scaled one by one and added in the order chosen, up to [MOST]. */
  private async add(files: readonly File[]): Promise<void> {
    this.pictureTrouble.set('');
    const room = MOST - this.chosen().length;
    if (files.length > room) {
      this.pictureTrouble.set(`a message carries ${MOST} pictures; the rest were left out`);
    }
    const added: Picture[] = [];
    for (const file of files.slice(0, Math.max(room, 0))) {
      try {
        const picture = await shrink(file);
        URL.revokeObjectURL(picture.preview);
        added.push(picture);
        this.telemetry.measured('picture', `${picture.width}x${picture.height} ${picture.bytes}B`);
      } catch (err: unknown) {
        this.pictureTrouble.set(`that image could not be read: ${reason(err)}`);
        this.telemetry.note('picture-refused', reason(err));
      }
    }
    // Read again: what was held may have changed while these were scaled.
    if (added.length)
      await this.drafts.hold(this.id(), [...this.chosen(), ...added].slice(0, MOST));
  }

  drop(index: number): void {
    void this.drafts.hold(
      this.id(),
      this.chosen().filter((_, at) => at !== index),
    );
    this.pictureTrouble.set('');
  }

  private dropAll(): void {
    void this.drafts.hold(this.id(), []);
    this.pictureTrouble.set('');
  }

  protected enlarge(picture: string): void {
    this.full.update((open) => (open === picture ? undefined : picture));
  }

  protected diff(change: Change): void {
    this.sheets.open(DiffSheet, { data: change, panelClass: 'session-sheet' });
  }

  protected parse(entry: ToolCall): void {
    this.sheets.open(ParseSheet, {
      data: {
        session: this.id(),
        command: entry.text,
        ok: entry.ok,
        hunks: entry.hunks,
        diverged: entry.diverged,
      },
      panelClass: 'session-sheet',
    });
  }

  /** A picture link inside rendered markdown opens the picture sheet. */
  protected tapped(event: MouseEvent): void {
    if (!(event.target instanceof Element)) return;
    const link = event.target.closest(`a.${PICTURE}`);
    const url = link && pointedAt(link.getAttribute('href') ?? '');
    if (!url) return;
    event.preventDefault();
    this.pictures.open(looking(this.api, url));
  }
}
