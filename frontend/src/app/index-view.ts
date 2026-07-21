import { Component, inject, signal } from '@angular/core';
import { MatProgressBarModule } from '@angular/material/progress-bar';

import { ContentNav } from './content-nav';
import { MemviewApi } from './memview-api';

/** The curated MEMORY.md index, rendered with links into /m/<name>. */
@Component({
  selector: 'app-index-view',
  templateUrl: './index-view.html',
  styleUrl: './index-view.scss',
  imports: [ContentNav, MatProgressBarModule],
})
export class IndexView {
  private api = inject(MemviewApi);

  readonly html = signal<string | null>(null);
  readonly count = signal(0);

  constructor() {
    this.api.index().subscribe({
      next: (page) => {
        this.html.set(page.html);
        this.count.set(page.count);
      },
      error: () => this.html.set('<p>No MEMORY.md index found.</p>'),
    });
  }
}
