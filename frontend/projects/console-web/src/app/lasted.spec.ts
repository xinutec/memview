import { describe, expect, it } from 'vitest';

import { Lasted } from './lasted';

const lasted = (ms: number | undefined): string => new Lasted().transform(ms);

describe('Lasted', () => {
  it('counts whole seconds under a minute', () => {
    // ⚠ No tenths. This repaints once a second, so a tenth would be wrong for
    // most of its life — unlike the turn line, which reports a finished
    // measurement and is right to be precise.
    expect(lasted(4_400)).toBe('4s');
    expect(lasted(59_900)).toBe('59s');
  });

  it('pads the seconds past a minute so the number does not shuffle', () => {
    // It is watched rather than read, and `2m 3s` becoming `2m 13s` moves every
    // character after it.
    expect(lasted(123_000)).toBe('2m 03s');
    expect(lasted(600_000)).toBe('10m 00s');
  });

  it('turns over to hours and minutes', () => {
    expect(lasted(3_900_000)).toBe('1h 05m');
  });

  it('says nothing about a thing with no start', () => {
    // A transcript line is entitled not to say when it happened, and an invented
    // duration is worse than none — the same rule the day markers follow.
    expect(lasted(undefined)).toBe('');
    expect(lasted(-1)).toBe('');
  });
});
