import { describe, expect, it } from 'vitest';

import { offered } from './rename-sheet';

describe('offered', () => {
  it('offers what the model wrote when the box says something else', () => {
    expect(offered('Lean port', '')).toBe('Lean port');
    expect(offered('Lean port', 'matcher work')).toBe('Lean port');
  });

  it('offers nothing when there is no suggestion', () => {
    // The ordinary case for a conversation with no gist yet, and for every gist
    // written before the second line was asked for. The sheet is what it was.
    expect(offered(undefined, '')).toBeUndefined();
  });

  it('offers nothing once the suggestion is what the box says', () => {
    // ⚠ Otherwise the button stays under a field it already filled, and a tap
    // that changes nothing reads as a broken control rather than a finished one.
    expect(offered('Lean port', 'Lean port')).toBeUndefined();
  });

  it('does not treat spacing as a different name', () => {
    // The box is being typed in; a trailing space is not a decision.
    expect(offered('Lean port', ' Lean port ')).toBeUndefined();
    expect(offered('  Lean port  ', 'Lean port')).toBeUndefined();
  });

  it('offers nothing when the model returned only spaces', () => {
    expect(offered('   ', '')).toBeUndefined();
    expect(offered('', '')).toBeUndefined();
  });

  it('offers nothing to a conversation a model would have named the same way', () => {
    // The box opens holding the name it has, so this falls out of the same
    // comparison — worth a case of its own because it is the one situation
    // where a suggestion is certainly useless rather than merely unwanted.
    expect(offered('Lean port', 'Lean port')).toBeUndefined();
  });
});
