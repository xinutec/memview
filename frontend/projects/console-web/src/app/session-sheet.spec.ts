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
  unread: 0,
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
    expect(labels(BARE)).not.toContain('last active');
  });

  it('opens with what the conversation is about, whole and attributed', () => {
    // The card clamps the sentence to two lines, so a long one is cut there —
    // and being cut is a reason to open this. Finding it cut here too would be
    // the panel failing at the one thing it is for.
    const long =
      'porting the last of the matcher gate to Lean, proving it bit-exact against the ' +
      'TypeScript twin, and running the golden set to see which journeys moved';
    const facts = factsOf(BARE, { text: long, at: new Date(2026, 7, 5, 8, 30).getTime() });

    expect(facts[0].label, 'it is the question the sheet is opened with').toBe('about');
    expect(facts[0].value).toBe(long);
    // ⚠ Said in words. Every other line here is read off a file or a process.
    expect(facts[0].note).toContain('written by Haiku');
  });

  it('says nothing about a conversation nothing has been written for', () => {
    // A sweep has not reached it, or the last one failed. An empty `about` row
    // would read as "this conversation is about nothing".
    expect(labels(BARE)).not.toContain('about');
  });

  it('does not claim what the session was started with', () => {
    // The console keeps the first prompt it heard, which for a resumed session
    // is the first one in the seeded page and for a long-running one is a job
    // finished days ago. A sheet is looked at to settle a question, so a line
    // that is usually wrong there is worse than no line.
    const session = { ...BARE, asked: 'Proceed' };
    expect(labels(session)).not.toContain('started with');
    expect(factsOf(session).map((fact) => fact.value)).not.toContain('Proceed');
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

  it('shows how full the context is beside how much has been said', () => {
    // The pair is the point: the first is what the model still has in front of
    // it, the second is everything ever said. A session two thirds full under a
    // 62 MB history has forgotten most of itself, and neither number says that
    // on its own.
    const session = { ...BARE, context: 640_000, window: 1_000_000, bytes: 65_011_712 };
    expect(value(session, 'context')).toBe('640k / 1M');
    expect(value(session, 'history')).toBe('62 MB');
  });

  it('says what the tokens would have cost, and calls it that', () => {
    // ⚠ Not `$4.21` on a card. It is not a bill — the session runs on the
    // subscription — and the flag that used to reveal it is account-wide while
    // the card was per-session, so it appeared beside whichever sessions were
    // talking when the API started warning.
    expect(value({ ...BARE, cost_usd: 422.883 }, 'tokens at list price')).toBe('$422.88');
    expect(labels({ ...BARE, cost_usd: 422.883 })).not.toContain('cost');
  });

  it('leaves it out for a session that has not spent anything', () => {
    expect(labels(BARE)).not.toContain('tokens at list price');
  });

  it('rounds a transcript with anything in it up to a megabyte', () => {
    // `0 MB` reads as an empty conversation where the truth is a short one.
    expect(value({ ...BARE, bytes: 4096 }, 'history')).toBe('1 MB');
  });

  it('leaves both out for a session that has not said', () => {
    // A session the console has just started has written nothing and answered
    // nothing. Blank rows here would be the em-dash problem again.
    expect(labels(BARE)).not.toContain('context');
    expect(labels(BARE)).not.toContain('history');
  });
});
