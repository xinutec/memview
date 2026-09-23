import { describe, expect, it } from 'vitest';

import { awake, pace } from './pace';

const HOUR = 3_600_000;

describe('pace', () => {
  it('is quiet near the end of a window with room left', () => {
    expect(pace(85, 0.93, 7 * HOUR)).toBe('even');
  });

  it('is amber when a quarter of the rest would not fit', () => {
    // 83% at 14:37 with a day and a half of the week left.
    expect(pace(83, 0.786, 21 * HOUR)).toBe('over');
  });

  it('is red when most of the rest would not fit', () => {
    expect(pace(95, 0.786, 21 * HOUR)).toBe('short');
    expect(pace(90, 0.744, 28 * HOUR)).toBe('short');
  });

  it('is quiet about a shortfall of minutes', () => {
    // 99% of five hours with twelve minutes left: nine of them would be dry.
    expect(pace(99, 0.96, 0.2 * HOUR)).toBe('even');
  });

  it('says so when the allowance would be left over', () => {
    expect(pace(50, 0.786, 21 * HOUR)).toBe('spare');
  });

  it('does not judge the first day', () => {
    expect(pace(40, 0.1, 88 * HOUR)).toBe('even');
  });

  it('does not judge a window that has closed', () => {
    expect(pace(99, 1, 0)).toBe('even');
  });
});

describe('awake', () => {
  it('counts only the waking hours of a day', () => {
    const midnight = new Date(2026, 8, 23).getTime();
    expect(awake(midnight, midnight + 24 * HOUR)).toBe(14 * HOUR);
  });

  it('counts part of a day from where it starts', () => {
    const from = new Date(2026, 8, 23, 14, 30).getTime();
    const to = new Date(2026, 8, 24, 9).getTime();
    expect(awake(from, to)).toBe(8.5 * HOUR);
  });

  it('counts nothing across a night', () => {
    const from = new Date(2026, 8, 23, 22).getTime();
    const to = new Date(2026, 8, 24, 8).getTime();
    expect(awake(from, to)).toBe(0);
  });
});
