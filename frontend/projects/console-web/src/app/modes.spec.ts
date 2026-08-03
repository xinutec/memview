import { describe, expect, it } from 'vitest';

import { MODES, modeIcon, modeIsLoud, modeTitle, offeredModes } from './modes';

describe('permission modes', () => {
  it('shows the CLI its own words, not a prettified guess', () => {
    // ⚠ `default` is displayed as *Manual*. Title-casing the stored name would
    // put a word on screen that appears nowhere in the tool the same person is
    // using — and would quietly disagree with it about what the session is
    // doing.
    expect(modeTitle('default')).toBe('Manual');
    expect(modeTitle('acceptEdits')).toBe('Accept edits');
    expect(modeTitle('auto')).toBe('Auto');
  });

  it('shows an unknown mode rather than hiding it', () => {
    // The CLI gains modes between releases. Dropping one this console has not
    // heard of would leave the header saying nothing at all about permissions —
    // which reads as the safe case, and is the one time it might not be.
    expect(modeTitle('somethingNew')).toBe('somethingNew');
  });

  it('says nothing for a session that has not recorded one', () => {
    expect(modeTitle(undefined)).toBeUndefined();
    expect(modeIsLoud(undefined)).toBe(false);
  });

  it('flags only the two the CLI itself colours as errors', () => {
    expect(modeIsLoud('bypassPermissions')).toBe(true);
    expect(modeIsLoud('dontAsk')).toBe(true);
    // `auto` lets a great deal through and is still not one of them — the
    // judgement here is the CLI's, not this console's.
    expect(modeIsLoud('auto')).toBe(false);
    expect(modeIsLoud('acceptEdits')).toBe(false);
  });

  it('ranks the modes by how much they let through', () => {
    // The CLI keeps this order; `plan` is the most restricted and
    // `bypassPermissions` the least.
    const ranks = Object.values(MODES).map((mode) => mode.rank);
    expect(Math.min(...ranks)).toBe(MODES['plan'].rank);
    expect(Math.max(...ranks)).toBe(MODES['bypassPermissions'].rank);
  });
});

describe('the menu of modes', () => {
  it('offers them least-allowed first, so the list reads as a dial', () => {
    // Sorted by the CLI's own rank rather than by however the table was
    // written: moving down the menu should always mean asking less.
    const order = offeredModes().map((mode) => mode.mode);
    expect(order[0]).toBe('plan');
    expect(order.at(-1)).toBe('bypassPermissions');
  });

  it('offers every mode the CLI knows about', () => {
    // A mode missing from the menu is one nobody can choose — and the runner
    // accepts all six, so the two would silently disagree.
    expect(offeredModes()).toHaveLength(Object.keys(MODES).length);
  });

  it('gives every mode an icon, since the header shows nothing else', () => {
    // The header has room for a glyph, not for "Bypass Permissions". A mode
    // without one would be invisible exactly where the answer matters.
    expect(offeredModes().every((mode) => mode.icon.length > 0)).toBe(true);
    expect(new Set(offeredModes().map((mode) => mode.icon)).size).toBe(offeredModes().length);
  });

  it('gives an unrecognised mode a visible icon rather than none', () => {
    // A blank where "what may this session do" goes reads as the careful
    // setting, which is the one case it might not be.
    expect(modeIcon('somethingNew')).toBe('help');
    expect(modeIcon(undefined)).toBeUndefined();
  });

  it('carries the warning through to the menu, where the choice is made', () => {
    const loud = offeredModes().filter((mode) => mode.loud);
    expect(loud.map((mode) => mode.mode).sort()).toEqual(['bypassPermissions', 'dontAsk']);
  });
});
