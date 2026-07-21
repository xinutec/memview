import { Routes } from '@angular/router';

import { AllView } from './all-view';
import { IndexView } from './index-view';
import { MemoryView } from './memory-view';
import { SearchView } from './search-view';
import { ShareEntry } from './share-entry';
import { SharingView } from './sharing-view';

/**
 * Routes for the SPA — a real table (fleet convention):
 *
 *   /              → MEMORY.md index (the curated map)
 *   /m/:name       → one memory, rendered
 *   /all           → every memory, grouped by type
 *   /search        → full-text search (?q=)
 *   /sharing       → owner-only share-link management
 *   /share/:token  → share-link landing: stores the token, then → /
 */
export const routes: Routes = [
  { path: '', component: IndexView },
  { path: 'm/:name', component: MemoryView },
  { path: 'all', component: AllView },
  { path: 'search', component: SearchView },
  { path: 'sharing', component: SharingView },
  { path: 'share/:token', component: ShareEntry },
  { path: '**', redirectTo: '' },
];
