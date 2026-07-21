import { Component, inject, signal } from '@angular/core';
import { MatButtonModule } from '@angular/material/button';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatToolbarModule } from '@angular/material/toolbar';
import { Router, RouterLink, RouterOutlet } from '@angular/router';

import { AuthStore } from './auth';
import { MemviewApi } from './memview-api';
import { Me } from './models';

@Component({
  selector: 'app-root',
  templateUrl: './app.html',
  styleUrl: './app.scss',
  imports: [RouterOutlet, RouterLink, MatToolbarModule, MatButtonModule, MatIconModule, MatProgressBarModule],
})
export class App {
  private api = inject(MemviewApi);
  private router = inject(Router);
  readonly auth = inject(AuthStore);

  readonly me = signal<Me | null>(null);
  readonly loading = signal(true);

  constructor() {
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
