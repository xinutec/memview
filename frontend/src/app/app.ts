import { Component, inject, signal } from '@angular/core';
import { MatButtonModule } from '@angular/material/button';
import { MatIconModule } from '@angular/material/icon';
import { MatMenuModule } from '@angular/material/menu';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatToolbarModule } from '@angular/material/toolbar';
import { Router, RouterLink, RouterOutlet } from '@angular/router';

import { AuthStore } from './auth';
import { MemviewApi } from './memview-api';
import { Telemetry } from './telemetry';
import { Me } from './models';

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
    MatProgressBarModule,
  ],
})
export class App {
  private api = inject(MemviewApi);
  private router = inject(Router);
  readonly auth = inject(AuthStore);
  private telemetry = inject(Telemetry);

  readonly me = signal<Me | null>(null);
  readonly loading = signal(true);

  constructor() {
    // Wired once, in the shell: two central seams (router events and one
    // capture-phase click listener), so no view knows this exists and no new
    // control can be missed by forgetting to annotate it.
    this.telemetry.init();
    this.api.me().subscribe({
      next: (me) => {
        this.me.set(me);
        this.loading.set(false);
      },
      // The interceptor already raised the sign-in wall on 401.
      error: () => this.loading.set(false),
    });
  }

  /** Post-login return target: wherever the user was heading. */
  loginHref(): string {
    return `/login?return_to=${encodeURIComponent(this.router.url)}`;
  }

  signOut(): void {
    this.api.logout().subscribe(() => (window.location.href = '/'));
  }
}
