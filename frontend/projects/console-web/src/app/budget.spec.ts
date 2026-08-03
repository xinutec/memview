import { describe, expect, it } from 'vitest';

import { costMatters } from './budget';

describe('costMatters', () => {
  it('says nothing while the account is inside its allowance', () => {
    // The case that prompted this: a subscription session showing `$3` it had
    // not been charged. Inside the allowance the number is not small, it is
    // meaningless.
    expect(costMatters('allowed')).toBe(false);
  });

  it('says nothing before the account has said anything', () => {
    // The common case — a session that has not yet been told. Hiding is the
    // right default because the failure to avoid is showing a false bill.
    expect(costMatters(undefined)).toBe(false);
  });

  it('speaks up as the allowance runs out', () => {
    // Warning, not just rejection: a number that appears only once you are
    // already blocked is too late to act on.
    expect(costMatters('allowed_warning')).toBe(true);
  });

  it('speaks up once the account is refusing work', () => {
    expect(costMatters('rejected')).toBe(true);
  });

  it('treats a status it does not know as not mattering', () => {
    // A later CLI may add one. Erring toward silence keeps the guarantee that
    // a visible figure always means something.
    expect(costMatters('some_future_status')).toBe(false);
  });
});
