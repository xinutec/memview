import { describe, expect, it } from 'vitest';

import { Task } from './models';
import { above, closedLabel, shownTasks, standingOf } from './tasks-sheet';

/**
 * The four states the service keeps, and the fifth it has not invented yet.
 *
 * `dropped` is closed without being done — overtaken, obsolete, decided
 * against. The sheet filtered on `status !== 'done'`, which was the same thing
 * as "open" right until that state existed: five dropped tasks then stood among
 * the open ones on the tasks session, wearing the unknown-status question mark,
 * while the toggle offered to reveal what it was already showing.
 */
const task = (id: string, status: string): Task => ({
  id,
  subject: `task ${id}`,
  status,
  detailed: false,
});

const ids = (tasks: readonly Task[]): string[] => tasks.map((t) => t.id);

describe('standingOf', () => {
  it('reads a dropped task as closed, and not as done', () => {
    expect(standingOf('dropped').open).toBe(false);
    expect(standingOf('dropped').title).toBe('dropped');
    // A tick would credit somebody with work nobody did, which is the whole
    // reason the service keeps the two states apart.
    expect(standingOf('dropped').icon).not.toBe(standingOf('done').icon);
  });

  it('still treats a state it has never heard of as news', () => {
    // The fallback that was doing dropped's job. It keeps its job for a fifth
    // state: shown, ranked with the open ones, marked as unknown.
    expect(standingOf('parked').open).toBe(true);
    expect(standingOf('parked').icon).toBe('help');
  });
});

describe('shownTasks', () => {
  const list = [
    task('1', 'done'),
    task('2', 'open'),
    task('3', 'dropped'),
    task('4', 'doing'),
    task('5', 'parked'),
  ];

  it('hides the dropped ones along with the done ones', () => {
    expect(ids(shownTasks(list, false))).toEqual(['4', '2', '5']);
  });

  it('shows them when asked, closed last and finished before abandoned', () => {
    expect(ids(shownTasks(list, true))).toEqual(['4', '2', '5', '1', '3']);
  });

  it('leaves the caller its own array', () => {
    // The signal holds this list; sorting it where it lies would reorder the
    // source of every other reading of it.
    const held = [...list];
    shownTasks(held, true);
    expect(ids(held)).toEqual(ids(list));
  });
});

describe('above', () => {
  it('lifts only the two ranks that outrank the untriaged', () => {
    expect(above('P0')).toBe(true);
    expect(above('P1')).toBe(true);
  });

  it('leaves P2 quiet, because P2 is where an unranked task already sits', () => {
    // Drawing it as urgent would say something about the row that is not true
    // of it: it is exactly as urgent as the four hundred nobody has read.
    expect(above('P2')).toBe(false);
  });

  it('leaves the two that sink below the untriaged quiet', () => {
    // "When there is room" and "not scheduled". A loud chip on these would make
    // the least pressing rows in the list the brightest ones in it.
    expect(above('P3')).toBe(false);
    expect(above('P4')).toBe(false);
  });

  it('draws no rank at all when there is none, and does not invent one', () => {
    // Absence is not a sixth level — it is P2's place, without the chip.
    expect(above(undefined)).toBe(false);
  });

  it('does not promote a level it has never heard of', () => {
    expect(above('P9')).toBe(false);
  });
});

describe('shownTasks and rank', () => {
  it('leaves the service’s order alone', () => {
    // ⚠ The service sorts, in `repo::list`, and P3 sinks BELOW everything
    // unranked — so the rows arrive with the low ranks last and that is
    // correct. A sort here would be a second rule to keep true, and it would
    // disagree with the first one the day either changed.
    const ranked = (id: string, priority?: string): Task => ({ ...task(id, 'open'), priority });
    const list = [ranked('748'), ranked('96', 'P3'), ranked('740', 'P3')];
    expect(ids(shownTasks(list, false))).toEqual(['748', '96', '740']);
  });

  it('still lifts what is underway, above even a P0 that has not been started', () => {
    // The one sort this sheet does keep, and it outranks the rank: the question
    // it is opened with is "what is this conversation actually on". So the
    // service's order survives WITHIN a status rather than across the list, and
    // a P0 floats above every other open task but not above work in hand.
    const ranked = (id: string, status: string, priority?: string): Task => ({
      ...task(id, status),
      priority,
    });
    const list = [ranked('1', 'open', 'P0'), ranked('2', 'doing')];
    expect(ids(shownTasks(list, false))).toEqual(['2', '1']);
  });
});

describe('closedLabel', () => {
  it('counts the dropped ones as closed too', () => {
    // The old label said "13 done" and hid five more behind the same toggle.
    const list = [task('1', 'done'), task('2', 'dropped'), task('3', 'open')];
    expect(closedLabel(list)).toBe('2 closed');
  });

  it('names the one kind when only one kind is there', () => {
    expect(closedLabel([task('1', 'done'), task('2', 'open')])).toBe('1 done');
    expect(closedLabel([task('1', 'dropped'), task('2', 'open')])).toBe('1 dropped');
  });

  it('says nothing when the toggle would reveal nothing', () => {
    // Empty is what hides the control: an inert switch is a question with one
    // answer. A session that has only dropped tasks must still get the toggle,
    // or its list looks empty and its rows are unreachable.
    expect(closedLabel([task('1', 'open'), task('2', 'doing')])).toBe('');
    expect(closedLabel([task('1', 'dropped')])).toBe('1 dropped');
  });
});
