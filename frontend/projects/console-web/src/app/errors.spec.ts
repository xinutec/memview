import { HttpErrorResponse } from '@angular/common/http';
import { describe, expect, it } from 'vitest';

import { reason } from './errors';

const failed = (status: number, error?: unknown) =>
  new HttpErrorResponse({ status, error, url: '/api/state' });

describe('reason — what went wrong, in words fit to put on screen', () => {
  it('prefers the runner’s own sentence to anything composed here', () => {
    // The runner answers failures in plain text — "…is not inside an allowed
    // directory" — and that is better than any category this could pick.
    expect(reason(failed(400, '  that path is not inside an allowed directory  '))).toBe(
      'that path is not inside an allowed directory',
    );
  });

  it('says there was no answer, and guesses at no device or cause', () => {
    // No answer says nothing of where it stopped: the client's network or key (its
    // window passing tracks waking hours), the way in, or the runner. Where it
    // stopped is measured instead — `Reach` and the runner's round trip.
    expect(reason(failed(0))).toBe('No answer from the runner.');
  });

  it('does not mistake an empty body for an explanation', () => {
    expect(reason(failed(0, '   '))).toBe('No answer from the runner.');
  });

  it('reports any other status as the runner answering', () => {
    expect(reason(failed(503))).toBe('The runner answered 503.');
  });

  it('takes unknown and narrows, rather than stringifying an object', () => {
    // This function exists because one callsite did `String(err)` and put
    // `cannot reach the runner: [object Object]` in front of the user — which
    // says nothing, and looks like a bug in the app rather than a sleeping Mac.
    expect(reason(new Error('the socket closed'))).toBe('the socket closed');
    expect(reason({ nothing: 'useful' })).toBe('Something went wrong.');
    expect(reason(undefined)).toBe('Something went wrong.');
  });
});
