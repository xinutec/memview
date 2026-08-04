import { describe, expect, it } from 'vitest';

import { fullness, tokens } from './tokens';

describe('tokens', () => {
  it('rounds to thousands, which is the precision a glance wants', () => {
    expect(tokens(496_231)).toBe('496k');
  });

  it('leaves a round million round', () => {
    // `1.0M` reads as a measurement to one decimal place; the window is exactly
    // a million and saying so is the point.
    expect(tokens(1_000_000)).toBe('1M');
  });

  it('gives a part million one decimal', () => {
    expect(tokens(1_640_000)).toBe('1.6M');
  });
});

describe('fullness', () => {
  it('reads as a fraction when the window is known', () => {
    expect(fullness(496_231, 1_000_000)).toBe('496k / 1M');
  });

  it('gives the count alone when it is not', () => {
    // A conversation that is not running, and a resumed one that has not
    // finished a turn: both know how full they are and not what they are full
    // of, because the window is declared on the CLI's result line and never in
    // the transcript. The count is still the number people watch for.
    expect(fullness(340_000)).toBe('340k');
  });

  it('says nothing when nothing has been reported', () => {
    // Not `0k`, which on screen is a claim about an empty context where this is
    // the absence of a measurement.
    expect(fullness(undefined, 1_000_000)).toBeUndefined();
    expect(fullness(0)).toBeUndefined();
  });
});
