import { TestBed } from '@angular/core/testing';
import { describe, expect, it } from 'vitest';

import { Usage } from './models';
import { UsageStrip } from './usage-strip';

const HOUR = 3_600_000;

function reading(usage: Partial<Usage> = {}): Usage {
  return {
    host: 'mac-mini',
    age_ms: 4 * HOUR,
    five_hour: { pct: 28, resets_in_ms: HOUR },
    seven_day: { pct: 66.4, resets_in_ms: 54 * HOUR },
    ...usage,
  };
}

/**
 * Render the strip and hand back its element.
 *
 * ⚠ **`nativeElement` is typed `any`**, and this project's lint refuses both an
 * unsafe member access and the assertion that would silence it. Narrowed
 * through `unknown` with an `instanceof` instead: that is a real check rather
 * than a promise to the compiler, and it fails loudly if the fixture ever stops
 * being an element. (`document.querySelector` does not work here — the TestBed
 * host is not attached to the page.)
 */
async function render(usage: Usage | undefined): Promise<Element> {
  await TestBed.configureTestingModule({ imports: [UsageStrip] }).compileComponents();
  const fixture = TestBed.createComponent(UsageStrip);
  fixture.componentRef.setInput('usage', usage);
  await fixture.whenStable();
  const host: unknown = fixture.nativeElement;
  if (!(host instanceof Element)) throw new Error('the fixture rendered no element');
  return host;
}

describe('the usage strip', () => {
  it('is not on the page at all until a reading has arrived', async () => {
    const host = await render(undefined);
    // Absent rather than empty or zeroed: a bar at 0% is a claim about the
    // account, and no reading is not a claim.
    expect(host.textContent?.trim()).toBe('');
  });

  it('says how much is spent and how long the window has left', async () => {
    const said = (await render(reading())).textContent ?? '';
    expect(said).toContain('28%');
    expect(said).toContain('1h');
    // Rounded, because a rate limit is not measured to a decimal place and a
    // figure that says 66.4% invites arithmetic it cannot support.
    expect(said).toContain('66%');
    expect(said).toContain('2d');
  });

  it('withholds a figure whose window has already turned over', async () => {
    // ⚠ The ordinary case here, not an edge one: the reading only refreshes when
    // an interactive session runs somewhere, so a console driven from a phone is
    // routinely looking at one taken several hours and one window ago.
    const said = (await render(reading({ five_hour: { pct: 28 } }))).textContent ?? '';
    expect(said).not.toContain('28%');
    expect(said).toContain('reset since');
    // And the longer window, still current, is untouched by it.
    expect(said).toContain('66%');
  });

  it('says how old the reading is and which machine took it', async () => {
    const said = (await render(reading({ age_ms: 4 * HOUR }))).textContent ?? '';
    expect(said).toContain('mac-mini');
    expect(said).toContain('4h ago');
  });

  it('does not shout about a window that has expired', async () => {
    // A full window that has already reset is not something to be alarmed by —
    // the alarm would be about a limit that has since been given back.
    const host = await render(reading({ five_hour: { pct: 99 } }));
    expect(host.querySelector('.high')).toBeNull();
  });

  it('marks a window that is nearly spent', async () => {
    const host = await render(reading({ five_hour: { pct: 92, resets_in_ms: HOUR } }));
    expect(host.querySelector('.pct.high')?.textContent).toContain('92%');
  });

  it("draws a model's own allowance under the model's name", async () => {
    const said =
      (await render(reading({ models: [{ model: 'Fable', pct: 6, resets_in_ms: 34 * HOUR }] })))
        .textContent ?? '';
    expect(said).toContain('Fable');
    expect(said).toContain('6%');
    // Beside the plan's own windows, not instead of them.
    expect(said).toContain('66%');
  });

  it('has no model row when the account has no model-scoped window', async () => {
    // The scopes come and go with Anthropic's plans — `seven_day_opus` and
    // `seven_day_sonnet` are both null now — so an empty list is ordinary and
    // must render as nothing rather than as a bar with no name.
    const said = (await render(reading({ models: [] }))).textContent ?? '';
    expect(said).toContain('66%');
    expect(said).not.toContain('undefined');
  });

  it('leaves out a window it has heard nothing about', async () => {
    // ⚠ Not the same as one that has reset. An event names one window at a
    // time, so the runner can know the week and have heard nothing yet about
    // the five hours — a row saying "reset since" would be an invented claim
    // about a window nobody has reported.
    const host = await render(reading({ five_hour: undefined }));
    const said = host.textContent ?? '';
    expect(said).not.toContain('5 hours');
    expect(said).not.toContain('reset since');
    expect(said).toContain('66%');
  });
});
