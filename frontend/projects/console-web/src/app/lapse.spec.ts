import { describe, expect, it } from 'vitest';

import { lapseWarning } from './lapse';

const HOUR = 3600_000;
const NOW = Date.UTC(2026, 10, 7, 12);

describe('lapseWarning', () => {
  it('says nothing while the lapse is more than three days off', () => {
    expect(lapseWarning(NOW + 18 * 24 * HOUR, NOW)).toBeUndefined();
    expect(lapseWarning(NOW + 72 * HOUR + 1, NOW)).toBeUndefined();
  });

  it('warns from three days out, in the largest whole unit', () => {
    expect(lapseWarning(NOW + 72 * HOUR, NOW)?.what).toContain('in 3 days');
    expect(lapseWarning(NOW + 30 * HOUR, NOW)?.what).toContain('in 30 hours');
    expect(lapseWarning(NOW + HOUR, NOW)?.what).toContain('in 1 hour;');
    expect(lapseWarning(NOW + 20 * 60_000, NOW)?.what).toContain('within the hour');
    expect(lapseWarning(NOW + HOUR, NOW)?.lapsed).toBe(false);
  });

  it('says signed out once it has lapsed', () => {
    const past = lapseWarning(NOW - 1, NOW);
    expect(past?.lapsed).toBe(true);
    expect(past?.lead).toBe('Signed out.');
  });

  it('says nothing when the runner could not read the lapse', () => {
    expect(lapseWarning(undefined, NOW)).toBeUndefined();
  });
});
