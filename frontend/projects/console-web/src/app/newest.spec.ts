import { describe, expect, it } from 'vitest';

import type { Summary } from './generated/Summary';
import { newest } from './session-view';

/** A summary of `id`, taken at `taken`; the rest does not matter here. */
function taken(id: string, at: number): Summary {
  return { id, taken: at } as Summary;
}

describe('newest', () => {
  it('keeps what is shown when an older word on the same session arrives', () => {
    // A poll asked before a send, answered after it: the message would vanish.
    const sent = taken('s', 2);
    expect(newest(sent, taken('s', 1))).toBe(sent);
  });

  it('takes a newer word', () => {
    const later = taken('s', 3);
    expect(newest(taken('s', 2), later)).toBe(later);
  });

  it('takes any word on another session, and the session going', () => {
    const other = taken('t', 1);
    expect(newest(taken('s', 2), other)).toBe(other);
    expect(newest(taken('s', 2), undefined)).toBeUndefined();
  });
});
