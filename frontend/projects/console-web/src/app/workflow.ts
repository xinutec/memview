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
 * The name a workflow goes by: a saved one's `name`, else the `name` its script's
 * `meta` literal declares, else the script file's, which the harness writes as
 * `<name>-<run id>.js`. The console labels running work the same way; see
 * `protocol::workflow_name`.
 */
export function workflowName(args: Readonly<Record<string, unknown>>): string | undefined {
  const { name, script, scriptPath } = args;
  if (typeof name === 'string') return name;
  if (typeof script === 'string') {
    return /meta[\s\S]*?\bname\s*:\s*(['"`])(.*?)\1/.exec(script)?.[2];
  }
  if (typeof scriptPath !== 'string') return undefined;
  const file = scriptPath.split('/').at(-1)?.replace(/\.js$/, '') ?? '';
  return file.replace(/-wf_[\w-]+$/, '');
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
  return running.some((called) => called.tool === 'Workflow' && called.task === task);
}

/** How often an open run, or an agent in it, is read again while it is going. */
export const REREAD_MS = 3000;
