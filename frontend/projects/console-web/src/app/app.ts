import { Component, inject } from '@angular/core';
import { MatButtonModule } from '@angular/material/button';
import { MatDividerModule } from '@angular/material/divider';
import { MatIconModule } from '@angular/material/icon';
import { MatMenuModule } from '@angular/material/menu';
import { MatToolbarModule } from '@angular/material/toolbar';
import { RouterLink, RouterOutlet } from '@angular/router';

import { BUILD_INFO } from './build-info';
import { ConsoleApi } from './console-api';
import { reason } from './errors';
import { Here } from './here';
import { offeredModes } from './modes';
import { Restyle } from './restyle';
import { Telemetry } from './telemetry';

@Component({
  selector: 'app-root',
  templateUrl: './app.html',
  styleUrl: './app.scss',
  imports: [
    RouterOutlet,
    RouterLink,
    MatToolbarModule,
    MatButtonModule,
    MatIconModule,
    MatMenuModule,
    MatDividerModule,
  ],
})
export class App {
  private telemetry = inject(Telemetry);
  private restyle = inject(Restyle);
  private api = inject(ConsoleApi);
  /** Read by the toolbar: the conversation on screen, when there is one. */
  readonly here = inject(Here);

  /** The permission modes to offer, least allowed first. See `modes.ts`. */
  protected readonly modes = offeredModes();

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

  protected stop(id: string): void {
    this.api.stop(id).subscribe({
      error: (err: unknown) => this.telemetry.note('stop-refused', reason(err)),
    });
  }
}
