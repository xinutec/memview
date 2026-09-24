import { NgTemplateOutlet } from '@angular/common';
import { Component, computed, effect, inject } from '@angular/core';
import { MatBottomSheet, MatBottomSheetModule } from '@angular/material/bottom-sheet';
import { MatButtonModule } from '@angular/material/button';
import { MatDividerModule } from '@angular/material/divider';
import { MatIconModule } from '@angular/material/icon';
import { MatMenuModule } from '@angular/material/menu';
import { MatSnackBar, MatSnackBarModule } from '@angular/material/snack-bar';
import { MatToolbarModule } from '@angular/material/toolbar';
import { RouterLink, RouterOutlet } from '@angular/router';

import { Awake } from './awake';
import { BUILD_INFO } from './build-info';
import { ConsoleApi } from './console-api';
import { Dismiss } from './dismiss';
import { reason } from './errors';
import { Here } from './here';
import { Summary } from './models';
import { type Known, modeIcon, modeIsLoud, modeTitle } from './modes';
import { titleOf } from './naming';
import { Choosing, ModesSheet } from './modes-sheet';
import { RenameSheet } from './rename-sheet';
import { Restyle } from './restyle';
import { SessionSheet } from './session-sheet';
import { JumpSheet, Where } from './jump-sheet';
import { TasksSheet } from './tasks-sheet';
import { Telemetry } from './telemetry';
import { SessionStore } from './session-store';

@Component({
  selector: 'app-root',
  templateUrl: './app.html',
  styleUrl: './app.scss',
  imports: [
    NgTemplateOutlet,
    RouterOutlet,
    RouterLink,
    MatToolbarModule,
    MatBottomSheetModule,
    MatButtonModule,
    MatIconModule,
    MatMenuModule,
    MatDividerModule,
    MatSnackBarModule,
  ],
})
export class App {
  private telemetry = inject(Telemetry);
  private store = inject(SessionStore);
  private restyle = inject(Restyle);
  private api = inject(ConsoleApi);
  private sheet = inject(MatBottomSheet);
  private dismiss = inject(Dismiss);
  private snack = inject(MatSnackBar);
  /** Read by the toolbar: the conversation on screen, when there is one. */
  readonly here = inject(Here);
  /** Read by the toolbar: whether the screen is being kept on. See [[Awake]]. */
  readonly awake = inject(Awake);

  /** What the open session may do without asking, for the one menu row that
   *  now stands in for the whole list. See [[ModesSheet]]. */
  protected readonly mode = computed(() => modeTitle(this.here.open()?.mode));
  protected readonly modeIcon = computed(() => modeIcon(this.here.open()?.mode));
  protected readonly loud = computed(() => modeIsLoud(this.here.open()?.mode));

  /**
   * What to call the conversation on screen — its name, else where it runs. A
   * `computed`, since a template method runs on every change-detection pass. The
   * same rule titles the list's cards; see `naming.ts`.
   */
  protected readonly title = computed(() => {
    const open = this.here.open();
    return open ? titleOf(open) : '';
  });

  /**
   * Whether the headline is standing in for a name rather than being one. Reads
   * `open`, not `at`: during the round trip there is no name and no stand-in either.
   */
  protected readonly anonymous = computed(() => {
    const open = this.here.open();
    return !!open && !open.name;
  });

  // Instrumented once, from the shell, so no new control can be missed.
  /**
   * Which build this page is, stamped into the bundle: a page cached in the
   * WebView must show its OWN age. `+` means an uncommitted tree.
   */
  protected readonly build = BUILD_INFO;
  protected readonly builtAt = new Date(BUILD_INFO.builtAt).toLocaleString();

  /**
   * The last refusal shown, so the poll does not repeat it every five seconds. The
   * words, not a flag: a different reason is worth saying.
   */
  private said?: string;

  constructor() {
    // Say why a mode change did not take, in the CLI's own words. The refusal
    // arrives on the poll, not on the request — the CLI answers on its own stream
    // later. A snack-bar because the menu is shut by then.
    effect(() => {
      const why = this.here.open()?.mode_refused;
      if (!why) {
        // Cleared server-side when another change is asked for.
        this.said = undefined;
        return;
      }
      if (why === this.said) return;
      this.said = why;
      this.telemetry.note('mode-refused', why);
      // At the top: the bottom of a session is the composer, and the default position
      // covered the text field and the send button for ten seconds.
      this.snack.open(why, 'ok', { duration: 10_000, verticalPosition: 'top' });
    });
    this.telemetry.init();
    // Before anything else on screen: an unstyled console shows `more_vert` where
    // its buttons were.
    this.restyle.init();
    // A decision from a previous visit, made once.
    this.awake.init();
  }

  /**
   * Ask the session to change what it may do without asking. Shown as chosen
   * before anything has confirmed it — a menu waiting on a phone round trip reads
   * as one that ignored the tap. Two things can still refuse it: the runner, in
   * the branch below, which puts the summary back; and the CLI, later, on its own
   * stream, which arrives on the poll as `mode_refused`.
   */
  protected setMode(mode: Known): void {
    const open = this.here.open();
    if (!open) return;
    this.here.open.set({ ...open, mode });
    this.api.setMode(open.id, mode).subscribe({
      error: (err: unknown) => {
        this.here.open.set(open);
        this.telemetry.note('mode-refused', reason(err));
      },
    });
  }

  /**
   * Everything about this session that the screen has no room for. Handed the
   * summary as it is NOW: the sheet is a still, and text that moves while it is
   * read is worse than text a second old.
   */
  protected details(session: Summary): void {
    this.dismiss.onBack(
      this.sheet.open(SessionSheet, {
        // The sentence travels beside the summary — see [[Here.gist]].
        data: { session, gist: this.here.gist() },
        panelClass: 'session-sheet',
      }),
    );
  }

  /**
   * How many of this session's tasks are still open, read off the poll. It used
   * to fetch the whole list when the menu opened — 63 kB for one session — and
   * could only answer for the conversation on screen. See `console/src/tasks.rs`.
   */
  protected readonly taskCount = this.here.tasks;

  /**
   * Offer everywhere in this conversation worth going back to, and go there. The
   * store does the jumping, since the transcript is the store's; the view lands at
   * the bottom of the page that arrives, off [[Held.adrift]].
   */
  protected goTo(session: Summary): void {
    const sheet = this.sheet.open<JumpSheet, Where, number>(JumpSheet, {
      data: { session: session.id },
      panelClass: 'session-sheet',
    });
    this.dismiss.onBack(sheet);
    sheet.afterDismissed().subscribe((at) => {
      if (at === undefined) return;
      this.store.goTo(session.id, at).subscribe({
        error: (failure: unknown) => this.telemetry.note('go-to-refused', reason(failure)),
      });
    });
  }

  protected tasks(session: Summary): void {
    this.dismiss.onBack(
      this.sheet.open(TasksSheet, {
        data: { session: session.id, name: session.name ?? undefined },
        panelClass: 'session-sheet',
      }),
    );
  }

  /** Offer what the session may do without asking. See [[ModesSheet]] for why
   *  this is a sheet rather than six rows in the menu. */
  protected chooseMode(session: Summary): void {
    // Typed on the way in, so the dismissal value is `Known | undefined`.
    const sheet = this.sheet.open<ModesSheet, Choosing, Known>(ModesSheet, {
      data: { id: session.id, mode: session.mode },
      panelClass: 'start-sheet',
    });
    this.dismiss.onBack(sheet);
    // The sheet chooses; this still records and rolls back, so one place knows what
    // the header is claiming.
    sheet.afterDismissed().subscribe((mode) => {
      if (mode) this.setMode(mode);
    });
  }

  /** Name the conversation. See [[RenameSheet]] for why the console does this
   *  itself instead of leaving it to `/rename`. */
  protected rename(session: Summary): void {
    this.dismiss.onBack(
      this.sheet.open(RenameSheet, {
        // The name it has, not what the list shows — prefilling `Code · 3f8a1c2b` is
        // prefilling something to delete. The suggestion goes beside the field, never
        // into it: it is a guess, and in the box the first Enter would make it a name.
        data: {
          id: session.id,
          title: session.name ?? '',
          suggestion: this.here.gist()?.name,
        },
        panelClass: 'start-sheet',
      }),
    );
  }

  protected stop(id: string): void {
    this.api.stop(id).subscribe({
      error: (err: unknown) => this.telemetry.note('stop-refused', reason(err)),
    });
  }
}
