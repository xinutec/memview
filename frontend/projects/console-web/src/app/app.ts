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
import { modeIcon, modeIsLoud, modeTitle } from './modes';
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
   * What to call the conversation on screen — its name, else where it runs.
   *
   * A `computed` rather than a method taking the session, because the template
   * reads it: a method body runs on every change-detection pass and cannot
   * cache (DL-ANGULAR-TEMPLATE-METHOD-CALL). The same rule titles the list's
   * cards; see `naming.ts`.
   */
  protected readonly title = computed(() => {
    const open = this.here.open();
    return open ? titleOf(open) : '';
  });

  /**
   * Whether the headline is standing in for a name rather than being one.
   *
   * Reads `open` and not `at`: during the round trip before the summary lands
   * there is no name and no stand-in either, and dimming an empty string says
   * nothing. See [[Here.at]].
   */
  protected readonly anonymous = computed(() => {
    const open = this.here.open();
    return !!open && !open.name;
  });

  // Instrumented once, from the shell: no screen knows the trace exists, so no
  // new control can be missed by forgetting to annotate it.
  /**
   * Which build this page is, stamped into the bundle rather than asked of the
   * server — a page cached in the WebView must show its OWN age, or the footer
   * would reassure with the server's current version while the reader looks at
   * something days older. `+` means it was built from an uncommitted tree.
   */
  protected readonly build = BUILD_INFO;
  protected readonly builtAt = new Date(BUILD_INFO.builtAt).toLocaleString();

  /**
   * The last refusal shown, so the poll does not show it every five seconds.
   *
   * The words themselves rather than a flag: a second refusal with a different
   * reason is worth saying, and the same one twice is not.
   */
  private said?: string;

  constructor() {
    // Say why a mode change did not take, in the CLI's own words.
    //
    // ⚠ **The refusal arrives on the poll, not on the request.** The console
    // writes the change to stdin and answers at once — the CLI's `error` comes
    // back on its own stream some time later — so `setMode`'s error branch below
    // sees only a request that could not be written. Until this existed the
    // header simply claimed the new mode for ever; see memview #96 and
    // `Session::settle_mode`.
    //
    // A snack-bar because the mode lives in a menu that is shut by the time the
    // answer comes: a correction nobody can see is the defect over again.
    effect(() => {
      const why = this.here.open()?.mode_refused;
      if (!why) {
        // Cleared server-side when another change is asked for, which is what
        // makes the same reason sayable again if it happens again.
        this.said = undefined;
        return;
      }
      if (why === this.said) return;
      this.said = why;
      this.telemetry.note('mode-refused', why);
      // ⚠ **At the top, which is not the default.** A snack-bar sits at the
      // bottom, and the bottom of a session is the composer — looked at on the
      // phone width, it covered the text field and the send button for the whole
      // ten seconds, so the answer to "your mode did not change" was "and now
      // you cannot type". The toolbar it covers instead is navigation, which is
      // not what somebody is doing at the moment a refusal arrives.
      this.snack.open(why, 'ok', { duration: 10_000, verticalPosition: 'top' });
    });
    this.telemetry.init();
    // Before anything else on screen: an unstyled console is one showing the
    // words `more_vert` and `send` where its buttons were.
    this.restyle.init();
    // Whether the screen was left held open is a decision from a previous visit,
    // and one nobody should have to make twice a day.
    this.awake.init();
  }

  /**
   * Ask the session to change what it may do without asking.
   *
   * ⚠ **Shown as chosen before anything has confirmed it.** A menu that waits
   * for a round trip over a phone connection reads as a menu that ignored the
   * tap. Two different things can still refuse it, and both are corrected now:
   *
   * * The **runner**, if it cannot write to the session — the branch below,
   *   which puts the summary back. It records the mode only once stdin has taken
   *   the line, so a failure leaves the true mode showing.
   * * The **CLI**, on its own stream some time later, which no amount of
   *   watching this request will show. That correction arrives on the poll as
   *   `mode_refused`, and the effect in the constructor is what says so.
   *   Until 2026-08-16 nothing read it and the claim stood for ever (#96).
   */
  protected setMode(mode: string): void {
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
   * Everything about this session that the screen has no room for.
   *
   * Takes the summary the template already narrowed rather than reading the
   * signal again, and hands it over as it is *now*: the sheet is a still, which
   * is what it is for. Nothing in it changes on the five-second poll except the
   * last-active time, and a panel whose text moves while it is being read from
   * is worse than one that is a second old.
   */
  protected details(session: Summary): void {
    this.dismiss.onBack(
      this.sheet.open(SessionSheet, {
        // The sentence travels beside the summary rather than on it — see
        // [[Here.gist]] — so the sheet is handed both.
        data: { session, gist: this.here.gist() },
        panelClass: 'session-sheet',
      }),
    );
  }

  /**
   * How many of this session's tasks are still open, and how many there are.
   *
   * ⚠ **Read off the poll, not counted here.** This used to fetch the session's
   * whole list when the menu opened — 63 kB for the session that keeps 355 tasks
   * — and could therefore only ever answer for the conversation already on
   * screen. The runner counts every session it can see in 1.4 ms of stats, so
   * the numbers now arrive with the state both screens already poll for, and the
   * list can show them too. See `console/src/tasks.rs`.
   */
  protected readonly taskCount = this.here.tasks;

  /**
   * Offer everywhere in this conversation worth going back to, and go there.
   *
   * The store rather than the view does the jumping, because the transcript is
   * the store's — the view follows it. What the view adds is landing at the
   * bottom of the page that arrives, which it does off [[Held.adrift]].
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
    // Typed on the way in, so the dismissal value is a `string | undefined`
    // rather than `any` — the sheet declares the same pair on its own ref.
    const sheet = this.sheet.open<ModesSheet, Choosing, string>(ModesSheet, {
      data: { id: session.id, mode: session.mode },
      panelClass: 'start-sheet',
    });
    this.dismiss.onBack(sheet);
    // The sheet chooses; this still records and rolls back, so there is exactly
    // one place that knows what the header is claiming.
    sheet.afterDismissed().subscribe((mode) => {
      if (mode) this.setMode(mode);
    });
  }

  /** Name the conversation. See [[RenameSheet]] for why the console does this
   *  itself instead of leaving it to `/rename`. */
  protected rename(session: Summary): void {
    this.dismiss.onBack(
      this.sheet.open(RenameSheet, {
        // The name it has, not what the list shows — a session that has never
        // been named shows `Code · 3f8a1c2b`, and prefilling that is prefilling
        // something to delete.
        //
        // ⚠ **The suggestion goes beside the field, never into it.** It is the
        // second line of the same Haiku call that writes the card's sentence, so
        // it is a guess about the conversation and the sheet draws it as one —
        // put in the box it would be indistinguishable from a name somebody
        // chose, and the first Enter would make it one.
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
