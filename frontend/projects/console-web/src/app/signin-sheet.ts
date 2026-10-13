import { Component, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';

import { ConsoleApi } from './console-api';
import { reason } from './errors';

/**
 * Sign the account in, whether the login has lapsed or is only getting old. One
 * login runs every session; the console runs `claude auth login`, whose link
 * shows a code to paste back (see `console::login`). Asked for on opening: a
 * sheet that offered a button to press before it could do anything would be one
 * step more.
 */
@Component({
  selector: 'app-signin-sheet',
  templateUrl: './signin-sheet.html',
  styleUrl: './signin-sheet.scss',
  imports: [FormsModule, MatButtonModule, MatFormFieldModule, MatIconModule, MatInputModule],
})
export class SignInSheet {
  private api = inject(ConsoleApi);

  /** The link to open, once the console has it. */
  protected readonly url = signal<string | undefined>(undefined);
  protected readonly code = signal('');
  protected readonly busy = signal(false);
  /** What the last step said, and whether it was good news. */
  protected readonly said = signal<{ ok: boolean; text: string } | undefined>(undefined);

  constructor() {
    this.ask();
  }

  /** A link, fresh: a refused code ends the attempt it belonged to. */
  protected ask(): void {
    this.busy.set(true);
    this.url.set(undefined);
    this.api.login().subscribe({
      next: ({ url }) => {
        this.busy.set(false);
        this.url.set(url);
      },
      error: (err: unknown) => {
        this.busy.set(false);
        this.said.set({ ok: false, text: reason(err) });
      },
    });
  }

  protected send(): void {
    const code = this.code().trim();
    if (!code || this.busy()) return;
    this.busy.set(true);
    this.said.set(undefined);
    this.api.loginCode(code).subscribe({
      next: ({ ok, said }) => {
        this.busy.set(false);
        this.url.set(undefined);
        this.code.set('');
        this.said.set(
          ok ? { ok, text: said || 'Signed in.' } : { ok, text: said || 'The code was refused.' },
        );
      },
      error: (err: unknown) => {
        this.busy.set(false);
        this.url.set(undefined);
        this.said.set({ ok: false, text: reason(err) });
      },
    });
  }
}
