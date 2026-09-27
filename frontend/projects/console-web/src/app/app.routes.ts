import { Routes } from '@angular/router';

import { AgentView } from './agent-view';
import { ReadingView } from './reading-view';
import { SessionsView } from './sessions-view';
import { SessionView } from './session-view';
import { WorkflowView } from './workflow-view';

export const routes: Routes = [
  { path: '', component: SessionsView },
  { path: 's/:id', component: SessionView, data: { up: { path: '/', label: 'all sessions' } } },
  // A workflow a session launched, and one of its agents. See [[WorkflowView]].
  { path: 's/:id/w/:run', component: WorkflowView, data: { up: '/s/:id' } },
  {
    path: 's/:id/w/:run/a/:agent',
    component: AgentView,
    data: { up: { path: '/s/:id/w/:run', keep: ['task'] } },
  },
  // Reached from the menu, never in the way of the list. See [[ReadingView]].
  {
    path: 'reader',
    component: ReadingView,
    data: { up: { path: '/', label: 'all sessions' } },
  },
  { path: '**', redirectTo: '' },
];
