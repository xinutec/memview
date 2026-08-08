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

describe('an unnamed session', () => {
  it('is told apart by its id, because the folder is the same for all of them', () => {
    // ⚠ Every session is started in `~/Code`, the parent of every repository, so
    // the folder answers `Code` for all of them and two new ones were
    // indistinguishable on the list.
    expect(
      titleOf({ dir: '/home/example/Code', id: '3f8a1c2b-0000-4000-8000-000000000001' }),
    ).toBe('Code · 3f8a1c2b');
  });

  it('keeps the name once it has one, id or no id', () => {
    expect(titleOf({ name: 'health', dir: '/home/example/Code', id: '3f8a1c2b' })).toBe('health');
  });

  it('says only the folder when there is no id to add', () => {
    // A conversation read off disk has no live session behind it.
    expect(titleOf({ dir: '/home/example/Code/memview' })).toBe('memview');
  });
});
