import { Component, inject, signal } from '@angular/core';
import { DatePipe } from '@angular/common';
import { MatButtonModule } from '@angular/material/button';
import { MatProgressBarModule } from '@angular/material/progress-bar';

import { MemviewApi } from './memview-api';
import { ShareInfo } from './models';

/**
 * Owner-only management of the public share link (the health app's share
 * mechanism): one token, rotation kills the old link instantly, revoke
 * removes access entirely.
 */
@Component({
  selector: 'app-sharing-view',
  templateUrl: './sharing-view.html',
  styleUrl: './sharing-view.scss',
  imports: [DatePipe, MatButtonModule, MatProgressBarModule],
})
export class SharingView {
  private api = inject(MemviewApi);

  readonly info = signal<ShareInfo | null>(null);
  readonly busy = signal(false);
  readonly copied = signal(false);

  constructor() {
    this.api.shareGet().subscribe((info) => this.info.set(info));
  }

  /** The pastable link: backend-configured base, else this origin. */
  shareUrl(info: ShareInfo): string {
    if (info.url) return info.url;
    return `${window.location.origin}/share/${info.token}`;
  }

  rotate(): void {
    this.busy.set(true);
    this.api.shareRotate().subscribe((info) => {
      this.info.set(info);
      this.busy.set(false);
      this.copied.set(false);
    });
  }

  revoke(): void {
    this.busy.set(true);
    this.api.shareRevoke().subscribe((info) => {
      this.info.set(info);
      this.busy.set(false);
      this.copied.set(false);
    });
  }

  copy(info: ShareInfo): void {
    void navigator.clipboard.writeText(this.shareUrl(info)).then(() => this.copied.set(true));
  }
}
