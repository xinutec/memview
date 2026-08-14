import { Routes } from '@angular/router';

import { AgentsView } from './agents-view';
import { AllView } from './all-view';
import { GraphView } from './graph-view';
import { IndexView } from './index-view';
import { MemoryView } from './memory-view';
import { SearchView } from './search-view';
import { ShareEntry } from './share-entry';
import { SharingView } from './sharing-view';
import { TimelineView } from './timeline-view';

/**
 * Routes for the SPA — a real table (fleet convention):
 *
 *   /              → MEMORY.md index (the curated map)
 *   /m/:name       → one memory, rendered
 *   /all           → every memory, grouped by type
 *   /graph         → the corpus as a 3D link graph (?metric=, ?walk=)
 *   /agents        → which named session works where (owner-only)
 *   /doing         → what they did, minute by minute, opening onto the evidence
 *   /search        → full-text search of the memories (?q=)
 *   /sharing       → owner-only share-link management
 *   /share/:token  → share-link landing: stores the token, then → /
 */
export const routes: Routes = [
  { path: '', component: IndexView },
  { path: 'm/:name', component: MemoryView },
  { path: 'all', component: AllView },
  { path: 'graph', component: GraphView },
  { path: 'agents', component: AgentsView },
  { path: 'doing', component: TimelineView },
  { path: 'search', component: SearchView },
  { path: 'sharing', component: SharingView },
  { path: 'share/:token', component: ShareEntry },
  { path: '**', redirectTo: '' },
];
