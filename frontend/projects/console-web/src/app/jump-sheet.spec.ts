import { describe, expect, it } from 'vitest';

import { Landmark, Mark } from './models';
import { byDay, signpostOf } from './jump-sheet';

/** Midday, so a day either side is a whole day away rather than an hour. */
const NOW = Date.UTC(2026, 7, 12, 12, 0, 0);
const DAY = 86_400_000;

const mark = (at: number, when: number | undefined, kind: Mark = 'prompt'): Landmark => ({
  at,
  when,
  kind,
  text: `said at ${at}`,
});

describe('byDay', () => {
  it('puts the newest day first, against the order the transcript has', () => {
    // ⚠ The file is oldest first and the console draws it that way. This list is
    // the other direction on purpose: what somebody wants back is far more often
    // this afternoon's than March's.
    const days = byDay([mark(10, NOW - 2 * DAY), mark(20, NOW - DAY), mark(30, NOW)], NOW);

    expect(days.map((day) => day.title)).toEqual([
      'Today',
      'Yesterday',
      new Date(NOW - 2 * DAY).toLocaleDateString(undefined, {
        weekday: 'short',
        day: 'numeric',
        month: 'short',
      }),
    ]);
  });

  it('puts the newest thing first inside a day too', () => {
    // Same argument one level down: a day with forty things said in it is
    // scrolled from the top, and the top should be the end of the afternoon.
    const [today] = byDay([mark(10, NOW - 3600_000), mark(20, NOW)], NOW);

    expect(today.marks.map((it) => it.at)).toEqual([20, 10]);
  });

  it('keeps what the transcript never dated, in a group of its own', () => {
    // ⚠ **Kept rather than dropped.** A line with no stamp is still somewhere to
    // jump to, and quietly losing it would make the strip incomplete in a way
    // nothing on screen could say. Never guessed a date for — that would file
    // June's conversation under today.
    const days = byDay([mark(10, undefined), mark(20, NOW)], NOW);

    expect(days.map((day) => day.title)).toEqual(['Today', 'Undated']);
    expect(days[1].marks.map((it) => it.at)).toEqual([10]);
  });

  it('has nothing to group when nothing survives the filter', () => {
    expect(byDay([], NOW)).toEqual([]);
  });
});

describe('signpostOf', () => {
  it('gives each kind its own mark', () => {
    // The kinds are what make the list scannable — a picture and a cut are the
    // two people navigate by, and one bullet for all four turns the strip back
    // into the paging it replaces.
    const icons = (['prompt', 'command', 'shown', 'compacted'] as Mark[]).map(
      (kind) => signpostOf(kind).icon,
    );

    expect(new Set(icons).size).toBe(icons.length);
  });

  it('has something to say for a compaction, which is a place and not a thing said', () => {
    // The runner sends an empty text for one. A row drawn from that alone would
    // be a tappable blank.
    expect(signpostOf('compacted').instead).not.toBe('');
  });

  it('draws a kind it has never heard of rather than nothing', () => {
    // A fifth landmark from a newer runner is news, not a reason for an empty
    // row — the same rule the task sheet's unknown status follows.
    expect(signpostOf('elsewhere' as Mark).icon).not.toBe('');
  });
});
