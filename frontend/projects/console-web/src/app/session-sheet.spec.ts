import { describe, expect, it } from 'vitest';

import { Summary } from './models';
import { factsOf } from './session-sheet';

/** The least a session can be: what the runner knows about one it has just
 *  started, before it has read a name, a model or a mode for it. */
const BARE: Summary = {
  id: '6f7c2f11-0000-4000-8000-000000000001',
  dir: '/home/example/Code/health',
  started: 1785600000,
  alive: true,
  interactions: 0,
  cost_usd: 0,
  waiting: 0,
};

const labels = (session: Summary): string[] => factsOf(session).map((fact) => fact.label);
const value = (session: Summary, label: string): string | undefined =>
  factsOf(session).find((fact) => fact.label === label)?.value;

describe('factsOf', () => {
  it('says where it is running, in full', () => {
    // The whole point of the sheet: the header shows the session's name, and the
    // path it stands for has to be somewhere.
    expect(value(BARE, 'where')).toBe('/home/example/Code/health');
  });

  it('says the session id, which is written nowhere else in the console', () => {
    expect(value(BARE, 'session id')).toBe(BARE.id);
  });

  it('leaves out what the runner has not read yet, rather than blanking it', () => {
    // A row of em-dashes reads as "this session has no model", where the truth
    // is "nobody has looked yet".
    expect(labels(BARE)).not.toContain('model');
    expect(labels(BARE)).not.toContain('permission mode');
    expect(labels(BARE)).not.toContain('started with');
    expect(labels(BARE)).not.toContain('last active');
  });

  it('gives the model id, not the short name the header shows', () => {
    const session = { ...BARE, model: 'claude-opus-5[1m]' };
    expect(value(session, 'model')).toBe('claude-opus-5[1m]');
  });

  it('spells the permission mode out, where the header has only its icon', () => {
    expect(value({ ...BARE, mode: 'bypassPermissions' }, 'permission mode')).toBe(
      'Bypass Permissions',
    );
  });

  it('keeps the rate limit off the sheet while nothing is wrong with it', () => {
    expect(labels({ ...BARE, limit: 'allowed' })).not.toContain('rate limit');
    expect(value({ ...BARE, limit: 'allowed_warning' }, 'rate limit')).toBe('allowed_warning');
  });

  it('dates a session from the second it was started, not from the epoch', () => {
    // `started` arrives in seconds and everything else in milliseconds; getting
    // that wrong dates every session to January 1970 and looks like a bug in the
    // runner rather than in this line.
    expect(value(BARE, 'started')).toContain(String(new Date(BARE.started * 1000).getFullYear()));
  });

  it('reports last activity from the transcript, in milliseconds', () => {
    const touched = new Date(2026, 6, 31, 8, 14).getTime();
    expect(value({ ...BARE, touched }, 'last active')).toContain(
      String(new Date(touched).getFullYear()),
    );
  });
});
