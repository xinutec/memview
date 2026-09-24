import { describe, expect, it } from 'vitest';

import { type Question, choiceOf, complete } from './questions';

/** A real `AskUserQuestion`, as the runner reads it. */
const ASKED: { questions: Question[] } = {
  questions: [
    {
      question: 'how far should the question UI go?',
      header: 'Scope',
      multiSelect: false,
      options: [
        { label: 'options only', description: 'render each option as a button' },
        { label: 'full parity', description: 'free text and notes as well' },
      ],
    },
  ],
};

describe('choiceOf', () => {
  it('says the label that was picked', () => {
    expect(choiceOf({ answers: { 'how far?': 'options only' } })).toBe('options only');
  });

  it('joins a multi-select without repeating the question', () => {
    // The question is still on screen above this line; saying it again turns a
    // one-line record into a paragraph on a phone.
    expect(choiceOf({ answers: { 'which?': ['the description', 'the topic'] } })).toBe(
      'the description, the topic',
    );
  });

  it('separates the answers to different questions', () => {
    expect(choiceOf({ answers: { one: 'left', two: 'north' } })).toBe('left · north');
  });

  it("prefers words over labels, which is the CLI's own precedence", () => {
    // A reply carrying both never leaves this app — the card makes them
    // exclusive — but one arriving from elsewhere should read the way the
    // session will read it.
    expect(choiceOf({ answers: { 'which?': 'left' }, response: 'neither, go back' })).toBe(
      'neither, go back',
    );
  });

  it('keeps a note beside the choice it qualifies', () => {
    expect(
      choiceOf({
        answers: { 'how far?': 'options only' },
        annotations: { 'how far?': { notes: 'but keep the skip button' } },
      }),
    ).toBe('options only (but keep the skip button)');
  });

  it('reports a note left against nothing, which is still an answer', () => {
    // The CLI records this one as `(no option selected) notes: …`, so a card
    // that showed nothing here would be quieter than the session's own record.
    expect(choiceOf({ annotations: { 'how far?': { notes: 'ask me again later' } } })).toBe(
      'ask me again later',
    );
  });

  it('has nothing to say about a reply that is not there', () => {
    expect(choiceOf(undefined)).toBe('');
    expect(choiceOf({})).toBe('');
    expect(choiceOf({ response: '   ' })).toBe('');
  });
});

describe('complete', () => {
  const questions = ASKED.questions;

  it('is not ready while nothing has been chosen', () => {
    expect(complete(questions, {})).toBe(false);
  });

  it('is ready once every question has an answer', () => {
    expect(complete(questions, { 'how far should the question UI go?': 'options only' })).toBe(
      true,
    );
  });

  it('counts a question answered by a note alone', () => {
    // The CLI accepts this and reports `(no option selected) notes: …`, so a
    // card that waited for a tap would sit grey over something sendable.
    expect(complete(questions, {}, { 'how far should the question UI go?': 'ask me later' })).toBe(
      true,
    );
  });

  it('does not count a blank note as one', () => {
    expect(complete(questions, {}, { 'how far should the question UI go?': '  ' })).toBe(false);
  });

  it('does not count an empty multi-select as answered', () => {
    // Tapping an option and then tapping it off again leaves the list there but
    // empty, which is not the same as having chosen nothing yet — and must not
    // enable a button that would send `[]`.
    expect(complete(questions, { 'how far should the question UI go?': [] })).toBe(false);
  });
});
