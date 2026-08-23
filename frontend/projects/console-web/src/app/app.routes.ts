import { Routes } from '@angular/router';

import { ReadingView } from './reading-view';
import { SessionsView } from './sessions-view';
import { SessionView } from './session-view';

export const routes: Routes = [
  { path: '', component: SessionsView },
  { path: 's/:id', component: SessionView },
  // Not about any session, and not urgent — reached from the menu, never in the
  // way of the list. See [[ReadingView]].
  { path: 'reader', component: ReadingView },
  { path: '**', redirectTo: '' },
];
