import { Routes } from '@angular/router';

import { AgentsView } from './agents-view';
import { AllView } from './all-view';
import { GraphView } from './graph-view';
import { IndexView } from './index-view';
import { MemoryView } from './memory-view';
import { ReaderView } from './reader-view';
import { SearchView } from './search-view';
import { TimelineView } from './timeline-view';

/**
 * Routes for the SPA — a real table (fleet convention). Every screen but the
 * index is drilled into from it, the menu's included, so each declares up to it
 * (@xinutec/ui-scaffold; Pippijn, 2026-09-27):
 *
 *   /              → MEMORY.md index (the curated map)
 *   /m/:name       → one memory, rendered
 *   /all           → every memory, grouped by type
 *   /graph         → the corpus as a 3D link graph (?metric=, ?walk=)
 *   /agents        → which named session works where
 *   /doing         → what they did, minute by minute, opening onto the evidence
 *   /reader        → what the reader makes of the fleet's shell, and what it cannot
 *   /search        → full-text search of the memories (?q=)
 */
export const routes: Routes = [
  { path: '', component: IndexView },
  { path: 'm/:name', component: MemoryView, data: { up: { path: '/', label: 'memory index' } } },
  { path: 'all', component: AllView, data: { up: { path: '/', label: 'memory index' } } },
  { path: 'graph', component: GraphView, data: { up: { path: '/', label: 'memory index' } } },
  { path: 'agents', component: AgentsView, data: { up: { path: '/', label: 'memory index' } } },
  { path: 'doing', component: TimelineView, data: { up: { path: '/', label: 'memory index' } } },
  { path: 'reader', component: ReaderView, data: { up: { path: '/', label: 'memory index' } } },
  { path: 'search', component: SearchView, data: { up: { path: '/', label: 'memory index' } } },
  { path: '**', redirectTo: '' },
];
