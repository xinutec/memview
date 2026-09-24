import type { Overview } from './models';

/** A workflow run as its launch names it. */
export interface Launched {
  /** The harness's task id, which the session's running work is listed under. */
  readonly task: string;
  /** The run id, which names the directory its agents write to. */
  readonly run: string;
}

/** The ids a `Workflow` call's result gives, when it launched one. */
export function launched(detail: string): Launched | undefined {
  const task = /^Workflow launched in background\. Task ID: (\w+)/.exec(detail)?.[1];
  const run = /^Run ID: (wf_[\w-]+)$/m.exec(detail)?.[1];
  return task && run ? { task, run } : undefined;
}

/**
 * Whether the run is still going: its task is among the session's running work.
 * `undefined` until the runner has answered.
 */
export function going(
  state: Overview | undefined,
  session: string,
  task: string,
): boolean | undefined {
  if (!state) return undefined;
  const running = state.sessions.find((one) => one.id === session)?.running ?? [];
  return running.some((called) => called.task === task);
}

/** How often an open run, or an agent in it, is read again while it is going. */
export const REREAD_MS = 3000;
