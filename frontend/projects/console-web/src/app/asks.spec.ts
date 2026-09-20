import { TestBed } from '@angular/core/testing';
import { Observable, throwError } from 'rxjs';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { Asks } from './asks';
import { ConsoleApi } from './console-api';
import { Here } from './here';
import type { Summary, Unanswered } from './models';
import type { Answers, Notes, Question } from './questions';

/**
 * What the approvals service does with a tap, and what it sends when it decides.
 *
 * Covered here rather than through the card because the risk is not the markup:
 * an answer sent twice, or sent with the wrong shape, is a session that acts on
 * something nobody meant. The render is covered at phone width in `e2e/`.
 */

/** One decision as it reached the runner. */
interface Sent {
  readonly at: string;
  readonly id: string;
  readonly allow: boolean;
  readonly answers?: Answers;
  readonly response?: string;
  readonly notes?: Notes;
}

function question(text: string, multiSelect = false): Question {
  return {
    question: text,
    header: 'Way',
    multiSelect,
    options: [
      { label: 'left', description: 'go left' },
      { label: 'right', description: 'go right' },
    ],
  };
}

/** An open ask, which is the only thing this service accepts. */
function open(questions?: readonly Question[]): Unanswered {
  return { kind: 'ask', ask: 'ask-1', tool: 'AskUserQuestion', text: 'which way', questions };
}

function harness(fail = false): { asks: Asks; sent: Sent[]; failing: { now: boolean } } {
  const sent: Sent[] = [];
  const failing = { now: fail };
  const api = {
    decide: vi.fn(
      (
        at: string,
        id: string,
        allow: boolean,
        _why?: string,
        answers?: Answers,
        response?: string,
        notes?: Notes,
      ): Observable<Summary> => {
        sent.push({ at, id, allow, answers, response, notes });
        return failing.now
          ? throwError(() => new Error('the tunnel went'))
          : new Observable<Summary>((s) => s.complete());
      },
    ),
  };
  TestBed.configureTestingModule({ providers: [{ provide: ConsoleApi, useValue: api }] });
  TestBed.inject(Here).at.set('session-1');
  return { asks: TestBed.inject(Asks), sent, failing };
}

describe('Asks', () => {
  beforeEach(() => TestBed.resetTestingModule());

  it('sends a single single-choice question on the tap', () => {
    // One question, one answer: on a phone this is answering from the lock
    // screen rather than putting it off, and it is the shape almost every
    // question has.
    const { asks, sent } = harness();
    const q = question('which way');
    asks.pick(open([q]), q, 'left');
    expect(sent).toHaveLength(1);
    expect(sent[0]?.answers).toEqual({ 'which way': 'left' });
    expect(sent[0]?.allow).toBe(true);
    expect(sent[0]?.id).toBe('ask-1');
  });

  it('holds a multi-select tap back and accumulates it', () => {
    const { asks, sent } = harness();
    const q = question('which ways', true);
    const entry = open([q]);
    asks.pick(entry, q, 'left');
    asks.pick(entry, q, 'right');
    expect(sent, 'a multi-select must wait for the send').toHaveLength(0);
    expect(asks.answers(open())).toEqual({ 'which ways': ['left', 'right'] });
  });

  it('takes a second tap on a chosen option as taking it back', () => {
    const { asks } = harness();
    const q = question('which ways', true);
    const entry = open([q]);
    asks.pick(entry, q, 'left');
    asks.pick(entry, q, 'left');
    expect(asks.answers(open())).toEqual({ 'which ways': [] });
  });

  it('holds two questions back until both are answered', () => {
    const { asks, sent } = harness();
    const one = question('which way');
    const two = question('how fast');
    const entry = open([one, two]);
    asks.pick(entry, one, 'left');
    expect(sent, 'sent with one of two answered').toHaveLength(0);
    asks.answer(entry);
    expect(sent, 'an incomplete answer was sent anyway').toHaveLength(0);
    asks.pick(entry, two, 'right');
    asks.answer(entry);
    expect(sent).toHaveLength(1);
    expect(sent[0]?.answers).toEqual({ 'which way': 'left', 'how fast': 'right' });
  });

  it('sends words instead of choices, and nothing else with them', () => {
    // ⚠ The CLI reads `response` BEFORE `answers` and takes only the one it
    // finds, so sending both would silently discard the choices. Typing takes
    // the card over; that is the behaviour, not a tidy-up.
    const { asks, sent } = harness();
    // Two questions, so the first tap does NOT send on its own and there is a
    // chosen answer standing when the words arrive.
    const one = question('which way');
    const entry = open([one, question('how fast')]);
    asks.pick(entry, one, 'left');
    asks.say(open(), '  neither, go back  ');
    asks.answer(entry);
    expect(sent).toHaveLength(1);
    expect(sent[0]?.response).toBe('neither, go back');
    expect(sent[0]?.answers, 'choices rode along with the words').toBeUndefined();
  });

  it('hands the card back when the typed words are cleared', () => {
    const { asks } = harness();
    asks.say(open(), 'wait');
    expect(asks.replying(open())).toBe(true);
    asks.say(open(), '   ');
    expect(asks.replying(open()), 'whitespace still counted as a reply').toBe(false);
  });

  it('refuses to tap an option while words are being typed', () => {
    const { asks, sent } = harness();
    const q = question('which way');
    asks.say(open(), 'hold on');
    asks.pick(open([q]), q, 'left');
    expect(sent).toHaveLength(0);
    expect(asks.answers(open())).toEqual({});
  });

  it('carries a note with the choice rather than instead of it', () => {
    const { asks, sent } = harness();
    const q = question('which way');
    const entry = open([q]);
    asks.jot(open(), q, 'because the left one is shorter');
    asks.pick(entry, q, 'left');
    expect(sent[0]?.answers).toEqual({ 'which way': 'left' });
    expect(sent[0]?.notes).toEqual({ 'which way': 'because the left one is shorter' });
  });

  it('allows and refuses an ask that offers no questions', () => {
    const { asks, sent } = harness();
    asks.decide(open(), false);
    expect(sent).toHaveLength(1);
    expect(sent[0]?.allow).toBe(false);
    expect(sent[0]?.answers).toBeUndefined();
  });

  it('keeps a note field open once it has been opened', () => {
    // It never closes on its own: half-typed reasoning vanishing under the
    // thumb is worse than a field nobody fills.
    const { asks } = harness();
    const q = question('which way');
    expect(asks.noteOpen(open(), q)).toBe(false);
    asks.openNote(open(), q);
    expect(asks.noteOpen(open(), q)).toBe(true);
  });

  it('keeps each ask’s answers to itself', () => {
    // Keyed by ask id because a re-seed destroys every card while the answers
    // worth keeping are exactly the ones tapped as the tunnel dropped.
    const { asks } = harness();
    const q = question('which way');
    asks.pick({ ...open([q, question('and back?')]) }, q, 'left');
    expect(asks.answers({ ask: 'ask-2' })).toEqual({});
  });

  it('reports a failed send instead of losing it', () => {
    const { asks } = harness(true);
    asks.decide(open(), true);
    expect(asks.trouble()).not.toBe('');
  });

  it('clears the last failure when the next attempt is made', () => {
    // Cleared on the attempt, not only on success: a stale sentence under a
    // question that has since gone through reads as still broken.
    const { asks, failing } = harness(true);
    asks.decide(open(), true);
    expect(asks.trouble()).not.toBe('');
    failing.now = false;
    asks.decide(open(), true);
    expect(asks.trouble()).toBe('');
  });

  it('sends nothing when no session is on screen', () => {
    // `Here.at` is the route's word for which conversation this is; without it
    // a decision has no address and must not be invented.
    const { asks, sent } = harness();
    TestBed.inject(Here).at.set(undefined);
    asks.decide(open(), true);
    expect(sent).toHaveLength(0);
  });
});
