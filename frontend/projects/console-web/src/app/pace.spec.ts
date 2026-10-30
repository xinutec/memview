import { describe, expect, it } from 'vitest';

import { awake, pace } from './pace';

const HOUR = 3_600_000;

describe('pace', () => {
  it('is quiet near the end of a window with room left', () => {
    expect(pace(85, 0.93, 7 * HOUR, true)).toBe('even');
  });

  it('is amber when a quarter of the rest would not fit', () => {
    // 83% at 14:37 with a day and a half of the week left.
    expect(pace(83, 0.786, 21 * HOUR, true)).toBe('over');
  });

  it('is red when most of the rest would not fit', () => {
    expect(pace(95, 0.786, 21 * HOUR, true)).toBe('short');
    expect(pace(90, 0.744, 28 * HOUR, true)).toBe('short');
  });

  it('is quiet about a shortfall of minutes', () => {
    // 99% of five hours with twelve minutes left: nine of them would be dry.
    expect(pace(99, 0.96, 0.2 * HOUR, false)).toBe('even');
  });

  it("is quiet on a spent week's last evening: too few hours would be without allowance", () => {
    // 98% at 18:12 and 100% at 21:22, 3h48m and 38m of waking week left.
    expect(pace(98, 1 - 3.8 / 98, 3.8 * HOUR, true)).toBe('even');
    expect(pace(100, 1 - 0.633 / 98, 0.633 * HOUR, true)).toBe('even');
  });

  it('is red only when a whole waking day would be without allowance', () => {
    // Most of the rest would not fit, but that is five hours, not a day.
    expect(pace(95, 0.9, 10 * HOUR, true)).toBe('over');
  });

  it('says so when the allowance would be left over', () => {
    expect(pace(50, 0.786, 21 * HOUR, true)).toBe('spare');
  });

  it('does not judge the first day', () => {
    expect(pace(40, 0.1, 88 * HOUR, true)).toBe('even');
  });

  it('does not judge a window that has closed', () => {
    expect(pace(99, 1, 0, true)).toBe('even');
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
