import { NgTemplateOutlet } from '@angular/common';
import { Component, computed, inject } from '@angular/core';
import { MatBottomSheet, MatBottomSheetModule } from '@angular/material/bottom-sheet';
import { MatButtonModule } from '@angular/material/button';
import { MatDividerModule } from '@angular/material/divider';
import { MatIconModule } from '@angular/material/icon';
import { MatMenuModule } from '@angular/material/menu';
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
  ],
})
export class App {
  private telemetry = inject(Telemetry);
  private store = inject(SessionStore);
  private restyle = inject(Restyle);
  private api = inject(ConsoleApi);
  private sheet = inject(MatBottomSheet);
  private dismiss = inject(Dismiss);
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

  constructor() {
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
   * ⚠ **Shown as chosen before the runner has confirmed it.** A menu that waits
   * for a round trip over a phone connection reads as a menu that ignored the
   * tap. The poll a few seconds later is what corrects it if the runner refused
   * — and the runner records the mode only once it has actually written the
   * request to the session, so a failure leaves the true mode showing there.
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
        data: { id: session.id, title: session.name ?? '' },
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
