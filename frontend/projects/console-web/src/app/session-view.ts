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

import { Composer } from './composer';
import { ConsoleApi } from './console-api';
import { Dismiss } from './dismiss';
import { Drafts } from './drafts';
import { EntryRow } from './entry-row';
import { reason } from './errors';
import { Folding } from './folding';
import { Following, measure } from './following';
import { Foreground } from './foreground';
import { Here } from './here';
import { Lasted } from './lasted';
import { modelName } from './model';
import { type Entry, type Summary, type ToolCall } from './models';
import { modeIcon, modeIsLoud, modeTitle } from './modes';
import { ParseSheet } from './parse-sheet';
import { PICTURE } from './rendered';
import { PictureSheet } from './picture-sheet';
import { pointedAt, shrink } from './picture';
import { Roster } from './roster';
import { Held, SessionStore } from './session-store';
import { Telemetry } from './telemetry';
import { fullness } from './tokens';
import { Block, Ran, blocks, ran } from './transcript';
import { Updates } from './updates';

/** One conversation: its transcript, what it is doing, and the box to reply in. */
@Component({
  selector: 'app-session-view',
  templateUrl: './session-view.html',
  styleUrl: './session-view.scss',
  host: { '(click)': 'tapped($event)' },
  imports: [Composer, EntryRow, Lasted, MatButtonModule, MatIconModule, MatProgressBarModule],
})
export class SessionView implements OnDestroy {
  readonly id = input.required<string>();

  private readonly api = inject(ConsoleApi);
  private readonly sheet = inject(MatBottomSheet);
  private readonly dismiss = inject(Dismiss);
  private readonly here = inject(Here);
  private readonly store = inject(SessionStore);
  private readonly drafts = inject(Drafts);
  private readonly roster = inject(Roster);
  private readonly telemetry = inject(Telemetry);
  private readonly foreground = inject(Foreground);
  private readonly until = inject(DestroyRef);
  private readonly injector = inject(Injector);

  readonly updateWaiting = inject(Updates).waiting;
  readonly unreachable = this.roster.unreachable;

  // What the store holds for this conversation, and what is derived from it.
  private readonly held = signal<Held | undefined>(undefined);
  readonly session = signal<Summary | undefined>(undefined);
  readonly entries = computed<Entry[]>(() => this.held()?.entries() ?? []);
  readonly blocks = computed<Block[]>(() => blocks(this.entries()));
  readonly more = computed(() => (this.held()?.cursor() ?? 0) > 0);
  readonly adrift = computed(() => this.held()?.adrift() ?? false);
  readonly stale = computed(() => this.held()?.stale() ?? false);
  /** The stream has been down long enough that what is below may be behind. */
  readonly dropped = computed(() => this.held()?.dropped() ?? false);
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
  readonly sending = signal(false);
  readonly trouble = signal('');
  readonly pictureTrouble = signal('');
  private readonly stored = toSignal(
    toObservable(this.id).pipe(switchMap((id) => this.drafts.text$(id))),
    { initialValue: undefined },
  );
  readonly picture = toSignal(
    toObservable(this.id).pipe(switchMap((id) => this.drafts.picture$(id))),
    { initialValue: undefined },
  );
  readonly reviving = signal(false);
  readonly unholding = signal<string | undefined>(undefined);
  readonly loading = signal(false);
  protected readonly full = signal<string | undefined>(undefined);
  protected readonly folding = new Folding();
  protected readonly pictureAt = (name: string): string => this.api.pictureAt(this.id(), name);

  // Scrolling: follow the end unless the reader has scrolled away.
  private readonly scroller = viewChild<ElementRef<HTMLElement>>('scroller');
  private readonly brink = viewChild<ElementRef<HTMLElement>>('brink');
  private readonly following = new Following();
  private readonly pages = signal(0);
  private readonly settled = signal(false);
  private was = 0;

  constructor() {
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

  protected shown(block: Block): readonly Entry[] {
    if (block.kind === 'one') return [block.entry];
    return this.folding.opensRun(block.key) ? block.entries : [];
  }

  protected counted(block: Block & { kind: 'tools' }): Ran {
    return ran(block.entries);
  }

  protected runningFor(block: Block & { kind: 'tools' }): number | undefined {
    const oldest = block.entries
      .filter((entry) => entry.ok === undefined && !entry.unrecorded && entry.at !== undefined)
      .map((entry) => entry.at ?? 0)
      .sort((a, b) => a - b)[0];
    return oldest === undefined ? undefined : this.now() - oldest;
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

  send(): void {
    const text = this.text().trim();
    const picture = this.picture();
    if ((!text && !picture) || this.sending()) return;
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
        this.trouble.set(reason(err));
      },
    });
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

  chose(file: File): void {
    this.pictureTrouble.set('');
    shrink(file)
      .then((picture) => {
        URL.revokeObjectURL(picture.preview);
        void this.drafts.hold(this.id(), picture);
        this.telemetry.measured('picture', `${picture.width}x${picture.height} ${picture.bytes}B`);
      })
      .catch((err: unknown) => {
        this.pictureTrouble.set(`that image could not be read: ${reason(err)}`);
        this.telemetry.note('picture-refused', reason(err));
      });
  }

  drop(): void {
    void this.drafts.hold(this.id(), undefined);
    this.pictureTrouble.set('');
  }

  protected enlarge(picture: string): void {
    this.full.update((open) => (open === picture ? undefined : picture));
  }

  protected parse(entry: ToolCall): void {
    this.dismiss.onBack(
      this.sheet.open(ParseSheet, {
        data: { session: this.id(), command: entry.text, ok: entry.ok },
        panelClass: 'session-sheet',
      }),
    );
  }

  /** A picture link inside rendered markdown opens the picture sheet. */
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
}
