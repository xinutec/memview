import { Routes } from '@angular/router';

import { SessionsView } from './sessions-view';
import { SessionView } from './session-view';

export const routes: Routes = [
  { path: '', component: SessionsView },
  { path: 's/:id', component: SessionView },
  { path: '**', redirectTo: '' },
];
