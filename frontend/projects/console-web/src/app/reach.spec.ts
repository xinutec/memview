import { describe, expect, it } from 'vitest';

import { Reach } from './reach';

const TROUBLE = 'cannot reach the runner: no answer — this phone may need unlocking';

describe('Reach — trouble has to outlive a poll before it is worth saying', () => {
  it('says nothing about a failure that is gone by the next poll', () => {
    // ⚠ The defect this exists for. Measured over 3.9 days of this console's
    // own telemetry: 235 status-0 episodes, median duration under a second,
    // which put "cannot reach the runner" on screen every 24 minutes for
    // something `consoleHost.renew()` had already fixed.
    const reach = new Reach();
    expect(reach.failed(TROUBLE)).toBe('');
    expect(reach.answered()).toBe('');
  });

  it('says it once the trouble has survived a second poll', () => {
    // All 26 episodes longer than five minutes clear this bar, which is the
    // property that matters: this hides blips, not failures.
    const reach = new Reach();
    reach.failed(TROUBLE);
    expect(reach.failed(TROUBLE)).toBe(TROUBLE);
  });

  it('keeps saying it while the trouble lasts', () => {
    const reach = new Reach();
    reach.failed(TROUBLE);
    reach.failed(TROUBLE);
    expect(reach.failed(TROUBLE)).toBe(TROUBLE);
  });

  it('forgets the failures once a poll answers', () => {
    // Two blips an hour apart are two blips, not one outage — otherwise the
    // count creeps up over a day and the banner appears on the first failure
    // of the evening.
    const reach = new Reach();
    reach.failed(TROUBLE);
    reach.answered();
    expect(reach.failed(TROUBLE)).toBe('');
  });

  it('clears what was on screen the moment a poll answers', () => {
    const reach = new Reach();
    reach.failed(TROUBLE);
    expect(reach.failed(TROUBLE)).toBe(TROUBLE);
    expect(reach.answered()).toBe('');
  });

  it('carries the caller’s wording rather than composing its own', () => {
    // The patience is this object's; the sentence belongs to `reason`, which is
    // where the one about the phone and the Mac is written and tested.
    const reach = new Reach();
    reach.failed('something else entirely');
    expect(reach.failed('something else entirely')).toBe('something else entirely');
  });
});
