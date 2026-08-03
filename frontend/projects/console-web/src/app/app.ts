import { Component, inject } from '@angular/core';
import { MatButtonModule } from '@angular/material/button';
import { MatIconModule } from '@angular/material/icon';
import { MatToolbarModule } from '@angular/material/toolbar';
import { RouterLink, RouterOutlet } from '@angular/router';

import { BUILD_INFO } from './build-info';
import { Here } from './here';
import { Restyle } from './restyle';
import { Telemetry } from './telemetry';

@Component({
  selector: 'app-root',
  templateUrl: './app.html',
  styleUrl: './app.scss',
  imports: [RouterOutlet, RouterLink, MatToolbarModule, MatButtonModule, MatIconModule],
})
export class App {
  private telemetry = inject(Telemetry);
  private restyle = inject(Restyle);
  /** Read by the toolbar: the conversation on screen, when there is one. */
  readonly here = inject(Here);

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
}
