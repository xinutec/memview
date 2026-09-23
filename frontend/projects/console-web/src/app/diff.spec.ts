import { describe, expect, it } from 'vitest';

import { lines } from './diff';

describe('lines', () => {
  it('marks what stayed, what went and what came', () => {
    const drawn = lines({
      before: 'a\nb\nc\n',
      after: 'a\nB\nc\n',
      everywhere: false,
      path: '/tmp/a',
    });
    expect(drawn).toEqual([
      { kind: 'same', text: 'a' },
      { kind: 'gone', text: 'b' },
      { kind: 'added', text: 'B' },
      { kind: 'same', text: 'c' },
    ]);
  });

  it('reads text without a final newline the same way', () => {
    expect(lines({ before: 'x', after: 'y', everywhere: false, path: '/tmp/a' })).toEqual([
      { kind: 'gone', text: 'x' },
      { kind: 'added', text: 'y' },
    ]);
  });

  it('draws an insertion as added lines only', () => {
    expect(lines({ before: '', after: 'new', everywhere: false, path: '/tmp/a' })).toEqual([
      { kind: 'added', text: 'new' },
    ]);
  });
});
