import { Component, OnDestroy, inject, signal } from '@angular/core';
import { MAT_BOTTOM_SHEET_DATA, MatBottomSheetRef } from '@angular/material/bottom-sheet';
import { MatButtonModule } from '@angular/material/button';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';

import { ConsoleApi } from './console-api';
import { reason } from './errors';

/** Which picture is being looked at: the address as the session wrote it. */
export interface Looking {
  readonly url: string;
}

/**
 * A picture a session pointed at, opened over the conversation.
 *
 * ⚠ **A lightbox here, where the sent-picture path deliberately has none.** A
 * picture sent from the phone is already in the transcript with the words about
 * it around it, so it opens in place — covering those words would take away the
 * reason for looking. This one is not in the transcript at all: there is nothing
 * to expand and nothing to cover, and what it usually holds is a render of a
 * room that is unreadable at a quarter of a phone screen.
 *
 * ⚠ **Back closes the viewer and only the viewer.** That is [[Dismiss]]'s
 * history entry, the same as every other sheet — without it, a back gesture
 * would close this *and* leave the conversation behind it, which is what
 * Material's `closeOnNavigation` does on its own.
 *
 * The bytes come through [[ConsoleApi.elsewhere]] rather than from an `<img
 * src>`, because the failures here are ordinary — the session's render server
 * outlives its links by minutes — and an `<img>` that fails says nothing about
 * why.
 */
@Component({
  selector: 'app-picture-sheet',
  templateUrl: './picture-sheet.html',
  styleUrl: './picture-sheet.scss',
  imports: [MatButtonModule, MatIconModule, MatProgressBarModule],
})
export class PictureSheet implements OnDestroy {
  private readonly given = inject<Looking>(MAT_BOTTOM_SHEET_DATA);
  private api = inject(ConsoleApi);
  private sheet = inject(MatBottomSheetRef<PictureSheet>);

  /** The address as it was written, which is how a person tells renders apart. */
  protected readonly url = this.given.url;
  /** The blob URL the picture is drawn from, once it has arrived. */
  protected readonly at = signal<string | undefined>(undefined);
  /** Why there is no picture, in the console's own words. */
  protected readonly trouble = signal('');
  /** Whether it is drawn at its own size rather than fitted to the screen. */
  protected readonly natural = signal(false);

  constructor() {
    this.api
      .elsewhere(this.given.url)
      .pipe(takeUntilDestroyed())
      .subscribe({
        next: (bytes) => this.at.set(URL.createObjectURL(bytes)),
        error: (err: unknown) => void this.explain(err),
      });
  }

  ngOnDestroy(): void {
    // ⚠ **A blob URL is a reference the document holds until it is revoked**, and
    // these are megabytes. Opening six renders in a conversation without this
    // keeps all six in the tab for as long as it lives.
    const at = this.at();
    if (at) URL.revokeObjectURL(at);
  }

  protected close(): void {
    this.sheet.dismiss();
  }

  /**
   * Fit to the screen, or the size it really is.
   *
   * The same gesture the sent pictures use, and for the same reason: a render of
   * a room fitted to a phone shows the shape and hides whether a wall came out
   * straight, which is the thing being looked for.
   */
  protected zoom(): void {
    this.natural.update((was) => !was);
  }

  /**
   * Why the picture did not arrive.
   *
   * ⚠ **Asking for a `Blob` means the failure arrives as one too.** The console
   * answers a failed fetch with a sentence — "it answered 404 Not Found", "what
   * came back is not a PNG… it begins `<!DOCTYPE html>`" — and with
   * `responseType: 'blob'` that sentence is a `Blob` on `err.error`, where
   * [[reason]] finds no string and falls back to "the runner answered 502". So
   * it is read out here, and [[reason]] answers everything else.
   */
  private async explain(err: unknown): Promise<void> {
    const body: unknown = err && typeof err === 'object' ? Reflect.get(err, 'error') : undefined;
    if (body instanceof Blob) {
      const said = (await body.text()).trim();
      if (said) {
        this.trouble.set(said);
        return;
      }
    }
    this.trouble.set(reason(err));
  }
}
