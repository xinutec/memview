import { describe, expect, it } from 'vitest';

import { placeOf, titleOf } from './naming';

describe('naming', () => {
  it('calls a session by its own name when it has one', () => {
    expect(titleOf({ name: 'health', dir: '/home/example/Code/health' })).toBe('health');
  });

  it('falls back to where it runs, which is the case every session starts in', () => {
    expect(titleOf({ dir: '/home/example/Code/memview' })).toBe('memview');
  });

  it('takes the last element of a deep path, not the first thing that differs', () => {
    expect(placeOf('/home/example/Code/health/packages/health-sync-backend/src/decode')).toBe(
      'decode',
    );
  });

  it('survives a trailing slash', () => {
    expect(placeOf('/home/example/Code/memview/')).toBe('memview');
  });

  it('says the path itself rather than nothing, for a root there is no name in', () => {
    expect(placeOf('/')).toBe('/');
  });
});
