import { Routes } from '@angular/router';

import { AgentView } from './agent-view';
import { ReadingView } from './reading-view';
import { SessionsView } from './sessions-view';
import { SessionView } from './session-view';
import { WorkflowView } from './workflow-view';

export const routes: Routes = [
  { path: '', component: SessionsView },
  { path: 's/:id', component: SessionView },
  // A workflow a session launched, and one of its agents. See [[WorkflowView]].
  { path: 's/:id/w/:run', component: WorkflowView },
  { path: 's/:id/w/:run/a/:agent', component: AgentView },
  // Reached from the menu, never in the way of the list. See [[ReadingView]].
  { path: 'reader', component: ReadingView },
  { path: '**', redirectTo: '' },
];
