import { Component, computed, inject, signal } from '@angular/core';
import { MatButtonModule } from '@angular/material/button';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { RouterLink } from '@angular/router';

import { MemviewApi } from './memview-api';
import { MemoryMeta } from './models';

/** Every memory, grouped by type, with a type filter. */
@Component({
  selector: 'app-all-view',
  templateUrl: './all-view.html',
  styleUrl: './all-view.scss',
  imports: [RouterLink, MatButtonModule, MatProgressBarModule],
})
export class AllView {
  private api = inject(MemviewApi);

  readonly memories = signal<MemoryMeta[] | null>(null);
  readonly filter = signal<string>('all');

  readonly types = computed(() => {
    const seen = new Set<string>();
    for (const m of this.memories() ?? []) seen.add(m.mtype);
    return [...seen].sort();
  });

  readonly visible = computed(() => {
    const f = this.filter();
    const list = this.memories() ?? [];
    return f === 'all' ? list : list.filter((m) => m.mtype === f);
  });

  constructor() {
    this.api.memories().subscribe((list) => this.memories.set(list));
  }
}
