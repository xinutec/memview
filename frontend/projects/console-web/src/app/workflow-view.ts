import {
  Component,
  DestroyRef,
  OnDestroy,
  computed,
  effect,
  inject,
  input,
  signal,
} from '@angular/core';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatProgressSpinnerModule } from '@angular/material/progress-spinner';
import { RouterLink } from '@angular/router';

import { ConsoleApi } from './console-api';
import { reason } from './errors';
import { Here, LIST } from './here';
import { type Agent, type Run, type ToolCall } from './models';
import { Roster } from './roster';
import { fold } from './transcript';
import { REREAD_MS, going } from './workflow';

/** What one agent is doing, as its row says it. */
type State = 'done' | 'working' | 'stopped';

/**
 * One workflow run: its phases, and under each the agents with what each is
 * doing. Read again while the run is going; an agent opens its transcript.
 */
@Component({
  selector: 'app-workflow-view',
  templateUrl: './workflow-view.html',
  styleUrl: './workflow-view.scss',
  imports: [MatIconModule, MatProgressBarModule, MatProgressSpinnerModule, RouterLink],
})
export class WorkflowView implements OnDestroy {
  readonly id = input.required<string>();
  readonly run = input.required<string>();
  /** The harness's task id for the run, from its launch. */
  readonly task = input.required<string>();
  readonly name = input<string>();

  private readonly api = inject(ConsoleApi);
  private readonly here = inject(Here);
  private readonly roster = inject(Roster);

  protected readonly got = signal<Run | undefined>(undefined);
  protected readonly trouble = signal<string | undefined>(undefined);
  protected readonly going = computed(() => going(this.roster.state(), this.id(), this.task()));

  protected readonly phases = computed(() =>
    (this.got()?.phases ?? []).map((phase) => ({
      title: phase.title ?? 'outside any phase',
      done: phase.agents.filter((agent) => agent.done).length,
      agents: phase.agents.map((agent) => ({
        agent,
        state: this.stateOf(agent),
        doing: doing(agent),
      })),
    })),
  );

  constructor() {
    inject(DestroyRef).onDestroy(this.roster.follow());
    effect(() => {
      this.here.page.set(this.name() ?? 'workflow');
      this.here.up.set({ path: `/s/${this.id()}` });
    });
    effect((onCleanup) => {
      const id = this.id();
      const run = this.run();
      const read = () =>
        this.api.workflow(id, run).subscribe({
          next: (got) => {
            this.got.set(got);
            this.trouble.set(undefined);
          },
          error: (wrong: unknown) => this.trouble.set(reason(wrong)),
        });
      read();
      if (this.going() === false) return;
      const timer = setInterval(read, REREAD_MS);
      onCleanup(() => clearInterval(timer));
    });
  }

  ngOnDestroy(): void {
    this.here.page.set(undefined);
    this.here.up.set(LIST);
  }

  private stateOf(agent: Agent): State {
    if (agent.done) return 'done';
    return this.going() === false ? 'stopped' : 'working';
  }
}

/** The agent's last call, as a transcript row would name it. */
function doing(agent: Agent): ToolCall | undefined {
  if (!agent.latest) return undefined;
  const [called] = fold([], agent.latest).filter((entry) => entry.kind === 'tool');
  return called;
}
