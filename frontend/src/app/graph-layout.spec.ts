import { describe, expect, it } from 'vitest';

import {
  Edge,
  SETTLED,
  createLayout,
  neighbourhood,
  project,
  sectionColour,
  stepLayout,
} from './graph-layout';

/** A rule cited by one project, and a second project two hops away through it. */
const NAMES = ['project_a', 'feedback_rule', 'project_b', 'reference_lonely'];
const EDGES: Edge[] = [
  { source: 'project_a', target: 'feedback_rule' },
  // Written the other way round on purpose: the rule cites project_b, rather
  // than project_b citing the rule. A directed walk would miss it.
  { source: 'feedback_rule', target: 'project_b' },
];

function distance(a: { x: number; y: number; z: number }, b: { x: number; y: number; z: number }) {
  return Math.hypot(a.x - b.x, a.y - b.y, a.z - b.z);
}

describe('neighbourhood', () => {
  it('includes the root and follows links in both directions', () => {
    const one = neighbourhood(EDGES, NAMES, 'project_a', 1);
    expect([...one].sort()).toEqual(['feedback_rule', 'project_a']);
  });

  it('reaches a memory that only ever gets cited, never cites', () => {
    // project_b writes no links at all; it is reachable only by walking the
    // rule's outgoing edge backwards. This is the "which rules govern this"
    // case — in the real corpus rules cite projects as often as the reverse.
    const two = neighbourhood(EDGES, NAMES, 'project_a', 2);
    expect([...two].sort()).toEqual(['feedback_rule', 'project_a', 'project_b']);
  });

  it('stops growing once the component is exhausted', () => {
    const far = neighbourhood(EDGES, NAMES, 'project_a', 99);
    expect(far.has('reference_lonely')).toBe(false);
    expect(far.size).toBe(3);
  });

  it('returns nothing for a name the graph does not have', () => {
    expect(neighbourhood(EDGES, NAMES, 'project_missing', 3).size).toBe(0);
  });

  it('gives an unlinked memory only itself', () => {
    expect([...neighbourhood(EDGES, NAMES, 'reference_lonely', 3)]).toEqual(['reference_lonely']);
  });
});

describe('createLayout', () => {
  it('places the same corpus identically twice', () => {
    // Deterministic on purpose: a reproducible picture makes a screenshot or a
    // bug report meaningful. Math.random would forfeit that.
    const a = createLayout(NAMES, EDGES);
    const b = createLayout(NAMES, EDGES);
    expect(a.nodes.map((n) => n.pos)).toEqual(b.nodes.map((n) => n.pos));
  });

  it('starts every node somewhere distinct', () => {
    const layout = createLayout(NAMES, EDGES);
    const seen = new Set(layout.nodes.map((n) => `${n.pos.x},${n.pos.y},${n.pos.z}`));
    expect(seen.size).toBe(NAMES.length);
  });

  it('drops an edge naming a node the graph does not have', () => {
    const layout = createLayout(NAMES, [...EDGES, { source: 'project_a', target: 'ghost' }]);
    expect(layout.pairs).toHaveLength(EDGES.length);
  });
});

describe('stepLayout', () => {
  it('cools to a stop and pulls linked memories closer than unlinked ones', () => {
    const layout = createLayout(NAMES, EDGES);
    for (let i = 0; i < 600; i++) stepLayout(layout);
    expect(layout.alpha).toBeLessThan(SETTLED);

    const at = (name: string) => {
      const node = layout.nodes[layout.index.get(name) ?? -1];
      expect(node, name).toBeDefined();
      return node.pos;
    };
    // project_a—feedback_rule are linked; reference_lonely is in no component,
    // so repulsion is the only force acting between it and anything else.
    const linked = distance(at('project_a'), at('feedback_rule'));
    const unlinked = distance(at('project_a'), at('reference_lonely'));
    expect(linked).toBeLessThan(unlinked);
    expect(Number.isFinite(linked)).toBe(true);
  });

  it('separates nodes that start on top of each other', () => {
    // A single-node corpus, then a degenerate two-node one: the guard against a
    // zero-distance inverse square must not produce NaN.
    const layout = createLayout(['a', 'b'], []);
    layout.nodes[0].pos = { x: 0, y: 0, z: 0 };
    layout.nodes[1].pos = { x: 0, y: 0, z: 0 };
    for (let i = 0; i < 50; i++) stepLayout(layout);
    for (const node of layout.nodes) {
      expect(Number.isFinite(node.pos.x + node.pos.y + node.pos.z)).toBe(true);
    }
    expect(distance(layout.nodes[0].pos, layout.nodes[1].pos)).toBeGreaterThan(0);
  });
});

describe('project', () => {
  const cam = { yaw: 0, pitch: 0, distance: 900 };

  it('puts the origin at the centre of the viewport', () => {
    const p = project({ x: 0, y: 0, z: 0 }, cam, 800, 600);
    expect(p.x).toBeCloseTo(400);
    expect(p.y).toBeCloseTo(300);
    expect(p.scale).toBeCloseTo(1);
  });

  it('shrinks what is further from the eye', () => {
    const near = project({ x: 0, y: 0, z: -200 }, cam, 800, 600);
    const far = project({ x: 0, y: 0, z: 200 }, cam, 800, 600);
    expect(near.scale).toBeGreaterThan(far.scale);
    expect(far.depth).toBeGreaterThan(near.depth);
  });

  it('keeps a point behind the eye on the far plane instead of mirroring it', () => {
    // Without the clamp, a negative depth flips the sign of the projection and
    // the node appears on the opposite side of the screen, in front.
    const behind = project({ x: 100, y: 0, z: -5000 }, cam, 800, 600);
    expect(behind.depth).toBeGreaterThan(0);
    expect(behind.x).toBeGreaterThan(400);
  });

  it('turns the scene with yaw', () => {
    const front = project({ x: 100, y: 0, z: 0 }, cam, 800, 600);
    const turned = project({ x: 100, y: 0, z: 0 }, { ...cam, yaw: Math.PI / 2 }, 800, 600);
    expect(turned.x).not.toBeCloseTo(front.x);
  });
});

describe('sectionColour', () => {
  it('gives neighbouring sections widely separated hues', () => {
    const hue = (i: number) => Number(/hsl\((\d+)/.exec(sectionColour(i))?.[1]);
    const gap = Math.abs(hue(0) - hue(1));
    expect(Math.min(gap, 360 - gap)).toBeGreaterThan(60);
  });

  it('is a literal colour canvas can parse, not a Material token', () => {
    // A `light-dark(...)` token assigned to fillStyle fails silently and leaves
    // the previous colour — invisible in light mode, black-on-black in dark.
    expect(sectionColour(3)).toMatch(/^hsl\(\d+ \d+% \d+%\)$/);
  });
});
