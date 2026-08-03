import {
  Component,
  DestroyRef,
  ElementRef,
  OnDestroy,
  effect,
  inject,
  input,
  signal,
  viewChild,
} from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';
import { MatProgressBarModule } from '@angular/material/progress-bar';

import { ConsoleApi } from './console-api';
import { reason } from './errors';
import { Foreground } from './foreground';
import { Entry, SessionEvent, Summary } from './models';
import { Rendered } from './rendered';
import { fold } from './transcript';

/** One session: what it has done, and the way to say something to it. */
@Component({
  selector: 'app-session-view',
  templateUrl: './session-view.html',
  styleUrl: './session-view.scss',
  imports: [
    FormsModule,
    MatButtonModule,
    MatIconModule,
    MatFormFieldModule,
    MatInputModule,
    MatProgressBarModule,
    Rendered,
  ],
})
export class SessionView implements OnDestroy {
  /** Bound from the route, so a session is a link. */
  readonly id = input.required<string>();

  private api = inject(ConsoleApi);
  private foreground = inject(Foreground);
  private until = inject(DestroyRef);
  private close?: () => void;
  private poll?: ReturnType<typeof setInterval>;

  // dev-lint: allow-component-list — `entries` is a fold of a live event stream
  // this component opens and closes, not a fetched catalog: retaining it without
  // the stream would show a conversation that had stopped being true. It is
  // reset by the effect that opens the stream and refilled from the stream —
  // which is the exemption this rule already grants, one method call out of
  // reach. Holding the stream and its fold in a root store is worth doing, and
  // is a different change: it would make re-entering a session resume rather
  // than re-seed, the way a reconnect now does.
  readonly entries = signal<Entry[]>([]);
  readonly session = signal<Summary | undefined>(undefined);
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
  /** Thinking is folded away by default: it is the longest thing on the page and
   *  the least often what the reader came for. */
  readonly showThinking = signal(false);
  readonly text = signal('');
  /** Whether anything older than what is on screen remains on disk. */
  readonly more = signal(false);
  readonly loading = signal(false);
  private scroller = viewChild<ElementRef<HTMLElement>>('scroller');

  constructor() {
    effect(() => {
      const id = this.id();
      this.close?.();
      this.forget();
      this.close = this.api.follow(
        id,
        (event) => this.take(event),
        // Only when the runner says the stream starts again — see [[ConsoleApi]].
        // Everything held has to go: it would otherwise be appended to by a
        // replay of itself, and there is no way to tell the two copies apart.
        () => this.forget(),
      );
      this.refresh();
      this.poll ??= setInterval(() => this.refresh(), 5000);
    });
    // The poll does not run while the phone is away, so the header facts on
    // screen when it comes back are as old as the pocket it was in. The
    // transcript below them heals itself — EventSource reconnects and replays
    // from the top — and these totals have nothing that would.
    this.foreground.onReturn(() => this.refresh(), this.until);
    // A frame after the entries change, not with them: `follow` reads a height
    // that does not exist until the browser has laid the new nodes out.
    // `afterRenderEffect` was the first thing tried here and never ran — proven
    // by the layout harness, which kept reporting scrollY 0.
    effect(() => {
      // Both, because either changes the height. Reading only the entries left
      // the view where it was when thinking was unfolded — which is the case the
      // layout harness tests, and the one that showed this was wrong.
      this.entries();
      this.showThinking();
      requestAnimationFrame(() => this.follow());
    });
  }

  /**
   * Drop everything held about the conversation on screen.
   *
   * All three, because they are one fact in three places: the entries, whether
   * anything older exists — which is re-established by the `joined` event the
   * new stream opens with — and whether the view has been placed yet, so the
   * fresh transcript opens at its newest message the way a first load does.
   */
  private forget(): void {
    this.entries.set([]);
    this.more.set(false);
    this.settled = false;
  }

  ngOnDestroy(): void {
    this.close?.();
    if (this.poll) clearInterval(this.poll);
  }

  /** The header facts — cost, turns, whether it is working — come from the
   *  summary rather than the stream, because they are totals and a client that
   *  reconnected mid-session has not seen every event that built them. */
  private refresh(): void {
    this.api.state().subscribe({
      next: (state) => {
        this.session.set(state.sessions.find((s) => s.id === this.id()));
        this.unreachable.set('');
      },
      error: (err: unknown) => this.unreachable.set(`cannot reach the runner: ${reason(err)}`),
    });
  }

  private take(event: SessionEvent): void {
    // The seed says how much it read; anything it read means there may be more
    // behind it. This is the only place that knows the conversation is longer
    // than the page.
    if (event.kind === 'joined' && (event.earlier ?? 0) > 0) this.more.set(true);
    this.entries.update((entries) => [...fold(entries, event)]);
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
   * all. `NEAR` is the slack — a few lines, so a partly-scrolled view still
   * counts as following.
   *
   */
  private follow(): void {
    const box = this.scroller()?.nativeElement;
    if (!box) return;
    const NEAR = 120;
    const atBottom = box.scrollHeight - box.scrollTop - box.clientHeight < NEAR;
    if (atBottom || !this.settled) {
      box.scrollTop = box.scrollHeight;
      this.settled = true;
    }
  }

  /** Whether the first render has happened; before it there is nothing to keep. */
  private settled = false;

  /**
   * The page before the one on screen.
   *
   * Counted in events held rather than by a cursor, because the file is the
   * authority and it grows: an offset taken now would name a different place
   * after the next turn. The runner re-reads and re-parses, which is why this is
   * on demand and not eager.
   */
  loadEarlier(): void {
    if (this.loading()) return;
    this.loading.set(true);
    const box = this.scroller()?.nativeElement;
    const before = box?.scrollHeight ?? 0;
    this.api.earlier(this.id(), this.entries().length).subscribe({
      next: (older) => {
        this.loading.set(false);
        this.more.set(older.more);
        // Folded on their own and put in front, rather than folded into the
        // list: fold joins an event to whatever precedes it, and an older page
        // has nothing before it here — appending would glue the top of the
        // conversation onto the bottom.
        let head: Entry[] = [];
        for (const event of older.events) head = [...fold(head, event)];
        this.entries.update((entries) => [...head, ...entries]);
        // Hold the reader's place: adding above them would otherwise move what
        // they were reading down the screen by the height of everything new.
        if (box) box.scrollTop += box.scrollHeight - before;
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
        this.text.set('');
        this.session.set(summary);
      },
      error: (err: unknown) => {
        this.sending.set(false);
        this.trouble.set(reason(err));
      },
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

  stop(): void {
    this.api.stop(this.id()).subscribe({
      next: (summary) => this.session.set(summary),
      error: (err: unknown) => this.trouble.set(reason(err)),
    });
  }

  visible(): Entry[] {
    const all = this.entries();
    return this.showThinking() ? all : all.filter((entry) => entry.kind !== 'thought');
  }

  thoughts(): number {
    return this.entries().filter((entry) => entry.kind === 'thought').length;
  }
}
