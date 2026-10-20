import { NgTemplateOutlet } from '@angular/common';
import { Component, computed, effect, inject } from '@angular/core';
import { MatBottomSheetModule } from '@angular/material/bottom-sheet';
import { MatButtonModule } from '@angular/material/button';
import { MatDividerModule } from '@angular/material/divider';
import { MatIconModule } from '@angular/material/icon';
import { MatMenuModule } from '@angular/material/menu';
import { MatSnackBar, MatSnackBarModule } from '@angular/material/snack-bar';
import { RouterLink, RouterOutlet } from '@angular/router';
import { Scaffold, ScaffoldActions, Sheets } from '@xinutec/ui-scaffold';

import { Awake } from './awake';
import { BUILD_INFO } from './build-info';
import { ConsoleApi } from './console-api';
import { reason } from './errors';
import { Here } from './here';
import { Summary } from './models';
import { type Known, modeIcon, modeIsLoud, modeTitle } from './modes';
import { Choosing, ModesSheet } from './modes-sheet';
import { RenameSheet } from './rename-sheet';
import { SignInSheet } from './signin-sheet';
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
    Scaffold,
    ScaffoldActions,
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
  private sheets = inject(Sheets);
  private snack = inject(MatSnackBar);
  /** Read by the toolbar: the conversation on screen, when there is one. */
  readonly here = inject(Here);
  /** Read by the toolbar: whether the screen is being kept on. See [[Awake]]. */
  readonly awake = inject(Awake);

  /** What the open session may do without asking, for the one menu row that
   *  stands in for the whole list. See [[ModesSheet]]. */
  protected readonly mode = computed(() => modeTitle(this.here.open()?.mode));
  protected readonly modeIcon = computed(() => modeIcon(this.here.open()?.mode));
  protected readonly loud = computed(() => modeIsLoud(this.here.open()?.mode));

  // Instrumented once, from the shell, so no new control can be missed.
  /**
   * Which build this page is, stamped into the bundle: a page cached in the
   * WebView must show its own age. `+` means an uncommitted tree.
   */
  protected readonly build = BUILD_INFO;
  protected readonly builtAt = new Date(BUILD_INFO.builtAt).toLocaleString();

  /**
   * The last refusal shown, so the poll does not repeat it every five seconds. The
   * words, not a flag: a different reason is worth saying.
   */
  private said?: string;

  constructor() {
    // Why a mode change did not take, in the CLI's words. It arrives on the poll,
    // after the menu has shut, hence a snack-bar.
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
      // At the top: at the bottom it would cover the composer.
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
   * Change what the session may do without asking. Shown as chosen at once, since
   * waiting on a round trip reads as an ignored tap. The runner can refuse it
   * (the branch below puts the summary back), and so can the CLI, later, as
   * `mode_refused` on the poll.
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
   * Everything about this session the screen has no room for, as it is now: text
   * that moves while read is worse than text a second old.
   */
  protected details(session: Summary): void {
    this.sheets.open(SessionSheet, {
      // The sentence travels beside the summary — see [[Here.gist]].
      data: { session, gist: this.here.gist() },
      panelClass: 'session-sheet',
    });
  }

  /** How many of this session's tasks are open, off the poll, not the large list. */
  protected readonly taskCount = this.here.tasks;

  /**
   * Offer the places in this conversation worth going back to, and go there. The
   * store jumps; the view lands at the bottom of the page, off [[Held.adrift]].
   */
  protected goTo(session: Summary): void {
    const sheet = this.sheets.open<JumpSheet, Where, number>(JumpSheet, {
      data: { session: session.id },
      panelClass: 'session-sheet',
    });
    sheet.afterDismissed().subscribe((at) => {
      if (at === undefined) return;
      this.store.goTo(session.id, at).subscribe({
        error: (failure: unknown) => this.telemetry.note('go-to-refused', reason(failure)),
      });
    });
  }

  protected tasks(session: Summary): void {
    this.sheets.open(TasksSheet, {
      data: { session: session.id, name: session.name ?? undefined },
      panelClass: 'session-sheet',
    });
  }

  /** Offer what the session may do without asking. See [[ModesSheet]] for why
   *  this is a sheet rather than six rows in the menu. */
  protected chooseMode(session: Summary): void {
    // Typed on the way in, so the dismissal value is `Known | undefined`.
    const sheet = this.sheets.open<ModesSheet, Choosing, Known>(ModesSheet, {
      data: { id: session.id, mode: session.mode },
      panelClass: 'start-sheet',
    });
    // The sheet chooses; this records and rolls back, so one place owns the header's claim.
    sheet.afterDismissed().subscribe((mode) => {
      if (mode) this.setMode(mode);
    });
  }

  /** Name the conversation. See [[RenameSheet]] for why the console does this
   *  itself instead of leaving it to `/rename`. */
  protected rename(session: Summary): void {
    this.sheets.open(RenameSheet, {
      // The name it has, not the list's `Code · 3f8a1c2b`. The suggestion goes beside
      // the field, not in it, or the first Enter would make a guess a name.
      data: {
        id: session.id,
        title: session.name ?? '',
        suggestion: this.here.gist()?.name,
      },
      panelClass: 'start-sheet',
    });
  }

  /** Sign the account in again. See [[SignInSheet]]. */
  protected signIn(): void {
    this.sheets.open(SignInSheet, { panelClass: 'start-sheet' });
  }

  protected stop(id: string): void {
    this.api.stop(id).subscribe({
      error: (err: unknown) => this.telemetry.note('stop-refused', reason(err)),
    });
  }
}
