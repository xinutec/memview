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

  it('names the phone before the Mac when there is no answer at all', () => {
    // The order is a finding, not a preference: status-0 failures track waking
    // hours, the opposite of what a sleeping Mac produces. The near cause is this
    // phone's client certificate passing its window.
    const said = reason(failed(0));
    expect(said).toContain('unlocking');
    // Both named before either is ordered — `indexOf` returns -1 for a word that
    // is missing, so the comparison below passes vacuously without these.
    expect(said).toContain('phone');
    // The Mac stays in the sentence: it is the case nobody would think of.
    expect(said).toContain('Mac');
    expect(said.indexOf('phone')).toBeLessThan(said.indexOf('Mac'));
  });

  it('does not mistake an empty body for an explanation', () => {
    expect(reason(failed(0, '   '))).toContain('unlocking');
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
