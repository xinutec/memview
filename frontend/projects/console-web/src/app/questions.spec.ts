import { describe, expect, it } from 'vitest';

import { choiceOf, complete, questionsOf } from './questions';

/** The shape a real `AskUserQuestion` arrives in, captured off the wire. */
const ASKED = {
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

describe('questionsOf', () => {
  it('reads a question the way it was asked', () => {
    const [question] = questionsOf(ASKED) ?? [];
    expect(question.question).toBe('how far should the question UI go?');
    expect(question.header).toBe('Scope');
    expect(question.multiSelect).toBe(false);
    expect(question.options.map((o) => o.label)).toEqual(['options only', 'full parity']);
    expect(question.options[0].description).toBe('render each option as a button');
  });

  it('says nothing about a tool call that asks no questions', () => {
    // Every other `ask` takes this path, and it is what keeps the ordinary
    // allow/refuse row for them.
    expect(questionsOf({ command: 'rm -rf /tmp/x' })).toBeUndefined();
    expect(questionsOf(undefined)).toBeUndefined();
    expect(questionsOf({ questions: [] })).toBeUndefined();
  });

  it('drops the whole ask rather than showing part of a question', () => {
    // ⚠ The case this exists for. A person choosing from a list cannot tell that
    // an option is missing, so a half-read question would be answered wrongly
    // with no way to notice. Falling back to allow/refuse is visibly less, and
    // wrong-looking beats wrong.
    const damaged = {
      questions: [
        {
          question: 'which way?',
          options: [{ label: 'left' }, { description: 'no label at all' }],
        },
      ],
    };
    expect(questionsOf(damaged)).toBeUndefined();
  });

  it('drops an ask where one question of several is unreadable', () => {
    const half = { questions: [ASKED.questions[0], { question: 'and?', options: [] }] };
    expect(questionsOf(half)).toBeUndefined();
  });

  it('treats a missing multiSelect as a single choice', () => {
    // The safe direction: guessing the other way would let one tap answer a
    // question that wanted several, and send before the person had finished.
    const [question] =
      questionsOf({ questions: [{ ...ASKED.questions[0], multiSelect: 'yes' }] }) ?? [];
    expect(question.multiSelect).toBe(false);
  });

  it('fills in a header that was not sent rather than dropping the question', () => {
    const [question] =
      questionsOf({ questions: [{ ...ASKED.questions[0], header: undefined }] }) ?? [];
    expect(question.header).toBe('');
  });
});

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

  it('has nothing to say about a reply that is not there', () => {
    expect(choiceOf(undefined)).toBe('');
    expect(choiceOf({})).toBe('');
    expect(choiceOf({ response: '   ' })).toBe('');
  });
});

describe('complete', () => {
  const questions = questionsOf(ASKED) ?? [];

  it('is not ready while nothing has been chosen', () => {
    expect(complete(questions, {})).toBe(false);
  });

  it('is ready once every question has an answer', () => {
    expect(complete(questions, { 'how far should the question UI go?': 'options only' })).toBe(
      true,
    );
  });

  it('does not count an empty multi-select as answered', () => {
    // Tapping an option and then tapping it off again leaves the list there but
    // empty, which is not the same as having chosen nothing yet — and must not
    // enable a button that would send `[]`.
    expect(complete(questions, { 'how far should the question UI go?': [] })).toBe(false);
  });
});
