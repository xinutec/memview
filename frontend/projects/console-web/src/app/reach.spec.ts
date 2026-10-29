import { TestBed } from '@angular/core/testing';
import { type Observable, of, throwError } from 'rxjs';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { ConsoleApi } from './console-api';
import { Local } from './local';
import type { Unreached } from './models';
import { Reach } from './reach';

/** The runner, as far as reports go: what it was told, and whether it answers. */
class Runner {
  readonly told: Unreached[][] = [];
  answering = true;
  reach(stretches: readonly Unreached[]): Observable<void> {
    this.told.push([...stretches]);
    return this.answering ? of(undefined) : throwError(() => new Error('no answer'));
  }
}

describe('Reach', () => {
  let runner: Runner;
  let database: string;

  function open(): Reach {
    TestBed.resetTestingModule();
    TestBed.configureTestingModule({ providers: [{ provide: ConsoleApi, useValue: runner }] });
    TestBed.inject(Local).under(database);
    return TestBed.inject(Reach);
  }

  beforeEach(() => {
    runner = new Runner();
    database = `t${Math.random().toString(36).slice(2)}`;
  });

  afterEach(() => TestBed.inject(Local).close());

  it('measures a stretch from the last answer to the next, and hands it over', async () => {
    const reach = open();
    await reach.answered(1_000);
    await reach.unanswered(6_000);
    await reach.unanswered(11_000);
    expect(reach.stretch()?.first_failure).toBe(6_000);
    await reach.answered(16_000);
    expect(reach.stretch()).toBeUndefined();
    expect(runner.told).toEqual([
      [
        {
          last_answer: 1_000,
          first_failure: 6_000,
          last_failure: 11_000,
          answered_again: 16_000,
          failures: 2,
        },
      ],
    ]);
  });

  it('keeps a stretch whose report was lost, and hands it over with the next answer', async () => {
    const reach = open();
    await reach.unanswered(6_000);
    runner.answering = false;
    await reach.answered(16_000);
    runner.answering = true;
    await reach.answered(21_000);
    expect(runner.told).toHaveLength(2);
    expect(runner.told[1]?.[0]?.answered_again).toBe(16_000);
    await reach.answered(26_000);
    expect(runner.told, 'reported once it was heard').toHaveLength(2);
  });

  it('finishes a stretch an earlier run left open', async () => {
    // Closed mid-stretch: its end lies between the last failure and this answer.
    const first = open();
    await first.unanswered(6_000);
    TestBed.inject(Local).close();
    const again = open();
    await again.answered(90_000);
    expect(runner.told[0]?.[0]).toMatchObject({ first_failure: 6_000, answered_again: 90_000 });
  });

  it('ends a stretch when the app goes to the background: pocket time is not an outage', async () => {
    const reach = open();
    await reach.answered(1_000);
    await reach.unanswered(6_000);
    await reach.away(8_000);
    await reach.unanswered(100_000);
    reach.back();
    await reach.answered(1_100_000);
    expect(runner.told).toEqual([
      [
        {
          last_answer: 1_000,
          first_failure: 6_000,
          last_failure: 6_000,
          answered_again: 8_000,
          failures: 1,
        },
      ],
    ]);
  });

  it('starts no stretch while the app is in the background', async () => {
    const reach = open();
    await reach.away(0);
    await reach.unanswered(5_000);
    expect(reach.stretch()).toBeUndefined();
  });
});
