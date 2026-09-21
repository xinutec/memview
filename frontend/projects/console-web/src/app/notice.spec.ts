import { describe, expect, it } from 'vitest';

import { notice } from './notice';

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
