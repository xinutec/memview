import { describe, expect, it } from 'vitest';

import { LONGEST, fitted, weight } from './picture';

describe('fitted', () => {
  it('brings a phone screenshot down to the edge worth sending', () => {
    // A Pixel 9 screenshot, which is the commonest thing this will ever carry.
    expect(fitted(1080, 2400)).toEqual({ width: 706, height: LONGEST });
  });

  it('brings a photograph down the other way round', () => {
    expect(fitted(4080, 3072)).toEqual({ width: LONGEST, height: 1181 });
  });

  it('leaves a small picture alone rather than blowing it up', () => {
    // ⚠ Enlarging costs four times the bytes for exactly the same picture — the
    // detail to read is not there to be recovered.
    expect(fitted(400, 300)).toEqual({ width: 400, height: 300 });
  });

  it('keeps the shape it was given', () => {
    const square = fitted(3000, 3000);
    expect(square.width).toBe(square.height);
  });
});

describe('weight', () => {
  it('says kB up to a megabyte and MB after it', () => {
    expect(weight(180 * 1024)).toBe('180 kB');
    expect(weight(2.5 * 1024 * 1024)).toBe('2.5 MB');
  });

  it('says bytes below a kilobyte rather than nothing', () => {
    // `0 kB` beside a thumbnail reads as a picture that failed to load.
    expect(weight(700)).toBe('700 B');
  });
});
