import {
  Component,
  DestroyRef,
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
import { MatBottomSheet } from '@angular/material/bottom-sheet';
import { MatButtonModule } from '@angular/material/button';
import { MatProgressBarModule } from '@angular/material/progress-bar';

import { ConsoleApi } from './console-api';
import { DiffSheet } from './diff-sheet';
import { Dismiss } from './dismiss';
import { reason } from './errors';
import { Folding } from './folding';
import { Here, LIST } from './here';
import { type Change, type Entry, type Timed, type ToolCall } from './models';
import { ParseSheet } from './parse-sheet';
import { Roster } from './roster';
import { TranscriptList } from './transcript-list';
import { type Block, blocks, fold } from './transcript';
import { REREAD_MS, going } from './workflow';

/** How close to the end the reader must be for new rows to keep the end in view. */
const NEAR_END_PX = 80;

/**
 * One workflow agent's transcript, read-only: the same rows as a session's, and
 * extended from where it was last read while its run is going.
 */
@Component({
  selector: 'app-agent-view',
  templateUrl: './agent-view.html',
  styleUrl: './agent-view.scss',
  imports: [MatButtonModule, MatProgressBarModule, TranscriptList],
})
export class AgentView implements OnDestroy {
  readonly id = input.required<string>();
  readonly run = input.required<string>();
  readonly agent = input.required<string>();
  readonly task = input.required<string>();
  readonly label = input<string>();

  private readonly api = inject(ConsoleApi);
  private readonly here = inject(Here);
  private readonly roster = inject(Roster);
  private readonly sheet = inject(MatBottomSheet);
  private readonly dismiss = inject(Dismiss);
  private readonly injector = inject(Injector);

  /** Everything read so far, oldest first, and the cursors on either side. */
  private readonly events = signal<readonly Timed[] | undefined>(undefined);
  private from = 0;
  private to = 0;
  protected readonly more = signal(false);
  protected readonly loading = signal(false);
  protected readonly trouble = signal<string | undefined>(undefined);

  protected readonly entries = computed<Entry[] | undefined>(() =>
    this.events()?.reduce<Entry[]>(fold, []),
  );
  protected readonly blocks = computed<Block[]>(() => blocks(this.entries() ?? []));
  protected readonly folding = new Folding();
  protected readonly pictureAt = (name: string): string => this.api.pictureAt(this.id(), name);

  private readonly list = viewChild(TranscriptList);
  private readonly scroller = computed(() => this.list()?.host);

  constructor() {
    inject(DestroyRef).onDestroy(this.roster.follow());
    effect(() => {
      this.here.page.set(this.label() ?? this.agent());
      this.here.up.set({
        path: `/s/${this.id()}/w/${this.run()}`,
        query: { task: this.task() },
      });
    });
    effect(() => {
      const [id, run, agent] = [this.id(), this.run(), this.agent()];
      untracked(() =>
        this.api.agent(id, run, agent).subscribe({
          next: (got) => {
            this.from = got.from;
            this.to = got.to;
            this.more.set(got.from > 0);
            this.events.set(got.events);
            this.trouble.set(undefined);
            this.toEnd();
          },
          error: (wrong: unknown) => this.trouble.set(reason(wrong)),
        }),
      );
    });
    effect((onCleanup) => {
      if (this.events() === undefined) return;
      if (going(this.roster.state(), this.id(), this.task()) === false) {
        untracked(() => this.extend());
        return;
      }
      const timer = setInterval(() => this.extend(), REREAD_MS);
      onCleanup(() => clearInterval(timer));
    });
  }

  ngOnDestroy(): void {
    this.here.page.set(undefined);
    this.here.up.set(LIST);
  }

  /** What the agent has done since it was last read. */
  private extend(): void {
    this.api.agent(this.id(), this.run(), this.agent(), { after: this.to }).subscribe({
      next: (got) => {
        const box = this.scroller()?.nativeElement;
        const near = !box || box.scrollHeight - box.scrollTop - box.clientHeight < NEAR_END_PX;
        this.to = got.to;
        this.trouble.set(undefined);
        if (!got.events.length) return;
        this.events.update((had) => [...(had ?? []), ...got.events]);
        if (near) this.toEnd();
      },
      error: (wrong: unknown) => this.trouble.set(reason(wrong)),
    });
  }

  protected earlier(): void {
    if (this.loading()) return;
    const box = this.scroller()?.nativeElement;
    const height = box?.scrollHeight ?? 0;
    this.loading.set(true);
    this.api.agent(this.id(), this.run(), this.agent(), { before: this.from }).subscribe({
      next: (got) => {
        this.loading.set(false);
        this.from = got.from;
        this.more.set(got.from > 0);
        this.events.update((had) => [...got.events, ...(had ?? [])]);
        afterNextRender(
          () => {
            if (box) box.scrollTop += box.scrollHeight - height;
          },
          { injector: this.injector },
        );
      },
      error: (wrong: unknown) => {
        this.loading.set(false);
        this.trouble.set(reason(wrong));
      },
    });
  }

  private toEnd(): void {
    afterNextRender(
      () => {
        const box = this.scroller()?.nativeElement;
        if (box) box.scrollTop = box.scrollHeight;
      },
      { injector: this.injector },
    );
  }

  protected diff(change: Change): void {
    this.dismiss.onBack(this.sheet.open(DiffSheet, { data: change, panelClass: 'session-sheet' }));
  }

  protected parse(entry: ToolCall): void {
    this.dismiss.onBack(
      this.sheet.open(ParseSheet, {
        data: { session: this.id(), command: entry.text, ok: entry.ok },
        panelClass: 'session-sheet',
      }),
    );
  }
}
