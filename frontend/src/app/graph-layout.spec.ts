import { describe, expect, it } from 'vitest';

import {
  Edge,
  LayoutInput,
  SETTLED,
  createLayout,
  fitZoom,
  neighbourhood,
  project,
  sectionColour,
  stepLayout,
} from './graph-layout';

/** A rule cited by one project, and a second project two hops away through it. */
const NAMES: LayoutInput[] = [
  { name: 'project_a', section: 'Projects' },
  { name: 'feedback_rule', section: 'Rules' },
  { name: 'project_b', section: 'Projects' },
  { name: 'reference_lonely', section: null },
];
const SECTIONS = ['Projects', 'Rules'];
const NAME_LIST = NAMES.map((n) => n.name);
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
    const one = neighbourhood(EDGES, NAME_LIST, 'project_a', 1);
    expect([...one].sort()).toEqual(['feedback_rule', 'project_a']);
  });

  it('reaches a memory that only ever gets cited, never cites', () => {
    // project_b writes no links at all; it is reachable only by walking the
    // rule's outgoing edge backwards. This is the "which rules govern this"
    // case — in the real corpus rules cite projects as often as the reverse.
    const two = neighbourhood(EDGES, NAME_LIST, 'project_a', 2);
    expect([...two].sort()).toEqual(['feedback_rule', 'project_a', 'project_b']);
  });

  it('stops growing once the component is exhausted', () => {
    const far = neighbourhood(EDGES, NAME_LIST, 'project_a', 99);
    expect(far.has('reference_lonely')).toBe(false);
    expect(far.size).toBe(3);
  });

  it('returns nothing for a name the graph does not have', () => {
    expect(neighbourhood(EDGES, NAME_LIST, 'project_missing', 3).size).toBe(0);
  });

  it('gives an unlinked memory only itself', () => {
    expect([...neighbourhood(EDGES, NAME_LIST, 'reference_lonely', 3)]).toEqual(['reference_lonely']);
  });
});

describe('createLayout', () => {
  it('places the same corpus identically twice', () => {
    // Deterministic on purpose: a reproducible picture makes a screenshot or a
    // bug report meaningful. Math.random would forfeit that.
    const a = createLayout(NAMES, EDGES, SECTIONS);
    const b = createLayout(NAMES, EDGES, SECTIONS);
    expect(a.nodes.map((n) => n.pos)).toEqual(b.nodes.map((n) => n.pos));
  });

  it('starts every node somewhere distinct', () => {
    const layout = createLayout(NAMES, EDGES, SECTIONS);
    const seen = new Set(layout.nodes.map((n) => `${n.pos.x},${n.pos.y},${n.pos.z}`));
    expect(seen.size).toBe(NAMES.length);
  });

  it('drops an edge naming a node the graph does not have', () => {
    const layout = createLayout(NAMES, [...EDGES, { source: 'project_a', target: 'ghost' }], SECTIONS);
    expect(layout.pairs).toHaveLength(EDGES.length);
  });
});

describe('stepLayout', () => {
  it('cools to a stop and pulls linked memories closer than unlinked ones', () => {
    const layout = createLayout(NAMES, EDGES, SECTIONS);
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

  it('gathers a section into its own territory even with no links at all', () => {
    // The force that stops the picture reading as a random scatter. Tested with
    // NO edges so nothing but the section pull can be responsible: if this ever
    // passes only because two nodes happen to be linked, it proves nothing.
    const inputs: LayoutInput[] = [
      { name: 'a1', section: 'A' },
      { name: 'a2', section: 'A' },
      { name: 'b1', section: 'B' },
      { name: 'b2', section: 'B' },
    ];
    const layout = createLayout(inputs, [], ['A', 'B']);
    for (let i = 0; i < 600; i++) stepLayout(layout);
    const at = (name: string) => layout.nodes[layout.index.get(name) ?? -1].pos;
    expect(distance(at('a1'), at('a2'))).toBeLessThan(distance(at('a1'), at('b1')));
    expect(distance(at('b1'), at('b2'))).toBeLessThan(distance(at('b1'), at('a2')));
  });

  it('leaves an unsectioned memory nearer the middle than a sectioned one', () => {
    // "Indexed under no heading" is depicted as floating unattached in the
    // centre rather than being given a home it does not have.
    const inputs: LayoutInput[] = [
      { name: 'loose', section: null },
      { name: 'a1', section: 'A' },
      { name: 'b1', section: 'B' },
    ];
    const layout = createLayout(inputs, [], ['A', 'B']);
    for (let i = 0; i < 600; i++) stepLayout(layout);
    const at = (name: string) => layout.nodes[layout.index.get(name) ?? -1].pos;
    const origin = { x: 0, y: 0, z: 0 };
    expect(distance(at('loose'), origin)).toBeLessThan(distance(at('a1'), origin));
  });

  it('separates nodes that start on top of each other', () => {
    // A single-node corpus, then a degenerate two-node one: the guard against a
    // zero-distance inverse square must not produce NaN.
    const layout = createLayout([{ name: 'a', section: null }, { name: 'b', section: null }], []);
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
  const cam = { yaw: 0, pitch: 0, distance: 900, zoom: 1 };

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

  it('scales the picture with zoom, not with camera distance', () => {
    // The bug this pins: with `scale = distance / depth`, the origin plane is
    // ALWAYS 1:1 (depth === distance there), so no camera distance could ever
    // frame a graph wider than the canvas and the fit was a silent no-op.
    const near = project({ x: 100, y: 0, z: 0 }, { ...cam, distance: 300 }, 800, 600);
    const far = project({ x: 100, y: 0, z: 0 }, { ...cam, distance: 3000 }, 800, 600);
    expect(near.x).toBeCloseTo(far.x);

    const out = project({ x: 100, y: 0, z: 0 }, { ...cam, zoom: 0.5 }, 800, 600);
    expect(out.x - 400).toBeCloseTo((near.x - 400) / 2);
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

describe('fitZoom', () => {
  it('frames the graph inside the smaller viewport axis with margin', () => {
    // 339 real memories settle to a radius of ~330 world units against a phone
    // canvas ~412px wide — unframed, that shows only the crowded middle and
    // clips every outer node.
    const zoom = fitZoom(330, 412, 620);
    expect(330 * zoom).toBeLessThan(412 / 2);
    expect(330 * zoom).toBeGreaterThan(412 / 2 / 1.5);
  });

  it('never divides by a zero-radius graph', () => {
    expect(Number.isFinite(fitZoom(0, 400, 400))).toBe(true);
  });
});
