import { describe, expect, it } from 'vitest';

import { REACH_FULL, curve, freshness, reach, standing, weight } from './graph-metrics';
import type { GraphNode, Usage } from './models';

const DAY = 86_400_000;
const NEWEST = Date.UTC(2026, 9, 20);

/** Only the fields scoring reads; the rest of a node is irrelevant here. */
function node(degree: number): GraphNode {
  return { in_degree: degree, out_degree: 0 } as GraphNode;
}

function usage(over: Partial<Usage> = {}): Usage {
  return { sessions: 0, turns: 0, reads: 0, edits: 0, last: null, projects: {}, ...over };
}

/** `n` projects, each mentioning the memory once. */
function projects(n: number): Record<string, number> {
  return Object.fromEntries(Array.from({ length: n }, (_, i) => [`p${i}`, 1]));
}

describe('reach', () => {
  it('draws a memory used in one project, or none, at zero', () => {
    // The first scaling drew a one-project memory at nearly half size.
    expect(reach(usage({ projects: projects(0) }))).toBe(0);
    expect(reach(usage({ projects: projects(1) }))).toBe(0);
  });

  it('reaches full size at REACH_FULL projects and clamps beyond', () => {
    expect(reach(usage({ projects: projects(REACH_FULL) }))).toBeCloseTo(1);
    expect(reach(usage({ projects: projects(REACH_FULL + 5) }))).toBe(1);
  });

  it('keeps the range between spread, not spent below six', () => {
    // The first scaling clamped everything from six projects up to the same 1.0.
    const six = reach(usage({ projects: projects(6) }));
    const ten = reach(usage({ projects: projects(10) }));
    expect(six).toBeGreaterThan(0.3);
    expect(six).toBeLessThan(ten);
    expect(ten).toBeLessThan(1);
  });
});

describe('freshness', () => {
  it('is full for the newest use and floors at a month', () => {
    const at = (days: number) => new Date(NEWEST - days * DAY).toISOString();
    expect(freshness(usage({ last: at(0) }), NEWEST)).toBe(1);
    expect(freshness(usage({ last: at(15) }), NEWEST)).toBeCloseTo(0.5);
    expect(freshness(usage({ last: at(90) }), NEWEST)).toBe(0.05);
  });

  it('draws a memory never seen small, not missing', () => {
    expect(freshness(usage(), NEWEST)).toBe(0.05);
    expect(freshness(usage({ last: new Date(NEWEST).toISOString() }), 0)).toBe(0.05);
  });
});

describe('standing', () => {
  it('composes the same curves the single metrics draw', () => {
    // So retuning one metric cannot leave `standing` on the old curve.
    const n = node(9);
    const u = usage({
      sessions: 4,
      reads: 9,
      projects: projects(3),
      last: new Date(NEWEST).toISOString(),
    });
    expect(standing(n, u, NEWEST)).toBeCloseTo(
      0.3 * curve.use(4) + 0.25 * curve.reads(9) + 0.2 * reach(u) + 0.15 * curve.links(9) + 0.1 * 1,
    );
  });

  it('cannot be carried to the top by one signal alone', () => {
    // Injected into every session but never opened, one project, long ago.
    const loud = usage({ sessions: 400, projects: projects(1) });
    expect(standing(node(0), loud, NEWEST)).toBeLessThan(0.5);
  });
});

describe('weight', () => {
  it('falls back to links when nothing has been mined, whatever is asked', () => {
    expect(weight(node(16), undefined, 'use', false, NEWEST)).toBe(curve.links(16));
    expect(weight(node(16), usage({ sessions: 100 }), 'reach', false, NEWEST)).toBe(
      curve.links(16),
    );
  });

  it('draws a memory with no signs of life small once anything is mined', () => {
    expect(weight(node(16), undefined, 'use', true, NEWEST)).toBe(0.05);
  });

  it('reads the metric asked for', () => {
    const u = usage({ sessions: 9, reads: 4, edits: 99 });
    expect(weight(node(0), u, 'use', true, NEWEST)).toBe(curve.use(9));
    expect(weight(node(0), u, 'reads', true, NEWEST)).toBe(curve.reads(4));
    expect(weight(node(0), u, 'edits', true, NEWEST)).toBe(curve.edits(99));
  });
});
