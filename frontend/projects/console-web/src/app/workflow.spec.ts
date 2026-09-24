import { describe, expect, it } from 'vitest';

import type { Overview } from './models';
import { going, launched } from './workflow';

describe('launched', () => {
  it('reads the task and run ids a launch gives', () => {
    const detail =
      'Workflow launched in background. Task ID: wpzk9pgqb\nSummary: Rewrite comments\nTranscript dir: /p/subagents/workflows/wf_0480c7f6-60f\nRun ID: wf_0480c7f6-60f\nTo resume: …';
    expect(launched(detail)).toEqual({ task: 'wpzk9pgqb', run: 'wf_0480c7f6-60f' });
  });

  it('is nothing for a result that only quotes a launch', () => {
    expect(launched('grep found: Workflow launched in background. Task ID: x')).toBeUndefined();
    expect(launched('Workflow launched in background. Task ID: wx')).toBeUndefined();
  });
});

describe('going', () => {
  const state = (running: { tool: string; task?: string }[]) =>
    ({ sessions: [{ id: 's', running }] }) as unknown as Overview;

  it("is the run's task among the session's running work", () => {
    expect(going(state([{ tool: 'Workflow', task: 'w1' }]), 's', 'w1')).toBe(true);
    expect(going(state([]), 's', 'w1')).toBe(false);
  });

  it('cannot say before the runner has answered', () => {
    expect(going(undefined, 's', 'w1')).toBeUndefined();
  });
});
