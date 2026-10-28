import { describe, expect, it } from 'vitest';

import { notice, worded } from './notice';

/**
 * Which of the three gets the line. There is one banner, so this ranking is the
 * whole answer to "why am I seeing that and not the other one".
 */
describe('notice', () => {
  const RUNNER = { kind: 'runner', what: 'no answer' } as const;
  const STREAM = { kind: 'stream' } as const;

  it('says nothing when nothing is wrong', () => {
    expect(notice({ acting: '', runner: undefined, stream: undefined })).toBeUndefined();
  });

  it('puts what the reader just pressed above any state', () => {
    // The more specific, and the one they caused — they are looking at it.
    expect(notice({ acting: 'the runner answered 503', runner: RUNNER, stream: STREAM })).toEqual({
      kind: 'acting',
      what: 'the runner answered 503',
    });
  });

  it('names the runner rather than the stream, when both are down', () => {
    // A stream that has stopped is what a runner that is not answering LOOKS
    // like. Saying "not live" there describes the symptom and leaves out the one
    // sentence that tells the reader what to try.
    expect(notice({ acting: '', runner: RUNNER, stream: STREAM })).toEqual(RUNNER);
  });

  it('names the stream when the runner is answering perfectly well', () => {
    // The case the stream marker exists for: the poll is healthy, so the list
    // looks fine, and the transcript has silently stopped.
    expect(notice({ acting: '', runner: undefined, stream: STREAM })).toEqual(STREAM);
  });
});

describe('worded', () => {
  it('says since when there has been no answer, and names no device or cause', () => {
    const at = new Date(2026, 9, 28, 14, 25).getTime();
    const said = worded({
      kind: 'runner',
      what: 'No answer from the runner.',
      since: at,
      answered: at - 5_000,
    });
    expect(said.lead).toMatch(/^No answer from the runner since .+\.$/);
    expect(said.what).toMatch(/^It last answered at .+\.$/);
    expect(`${said.lead} ${said.what}`).not.toMatch(/phone|Mac|asleep|unlock/i);
  });

  it('passes on an error the runner answered with', () => {
    expect(worded({ kind: 'runner', what: 'The runner answered 500.' })).toMatchObject({
      what: 'The runner answered 500.',
    });
  });
});
