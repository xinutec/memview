import { describe, expect, it } from 'vitest';

import {
  boundingRadius,
  bridges,
  clusterLevels,
  companionsOf,
  createLayout,
  Edge,
  fitZoom,
  frameFor,
  LABEL_BUDGET,
  LabelCandidate,
  LayoutInput,
  MIN_FRAME_RADIUS,
  neighbourhood,
  neighboursOf,
  panDelta,
  planLabels,
  project,
  sectionColour,
  SETTLED,
  stepLayout,
} from './graph-layout';

/** A rule cited by one project, and a second project two hops away through it. */
const NAMES: LayoutInput[] = [
  { name: 'project_a', group: 'Projects' },
  { name: 'feedback_rule', group: 'Rules' },
  { name: 'project_b', group: 'Projects' },
  { name: 'reference_lonely', group: null },
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

  it('gathers a group into its own territory even with no links at all', () => {
    // The force that stops the picture reading as a random scatter. Tested with
    // NO edges so nothing but the group pull can be responsible: if this ever
    // passes only because two nodes happen to be linked, it proves nothing.
    const inputs: LayoutInput[] = [
      { name: 'a1', group: 'A' },
      { name: 'a2', group: 'A' },
      { name: 'b1', group: 'B' },
      { name: 'b2', group: 'B' },
    ];
    const layout = createLayout(inputs, [], ['A', 'B']);
    for (let i = 0; i < 600; i++) stepLayout(layout);
    const at = (name: string) => layout.nodes[layout.index.get(name) ?? -1].pos;
    expect(distance(at('a1'), at('a2'))).toBeLessThan(distance(at('a1'), at('b1')));
    expect(distance(at('b1'), at('b2'))).toBeLessThan(distance(at('b1'), at('a2')));
  });

  it('leaves an ungrouped memory nearer the middle than a grouped one', () => {
    // A memory no cluster claimed is depicted as floating unattached in the
    // centre rather than being given a home it does not have.
    const inputs: LayoutInput[] = [
      { name: 'loose', group: null },
      { name: 'a1', group: 'A' },
      { name: 'b1', group: 'B' },
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
    const layout = createLayout([{ name: 'a', group: null }, { name: 'b', group: null }], []);
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
  const cam = { yaw: 0, pitch: 0, distance: 900, zoom: 1, target: { x: 0, y: 0, z: 0 } };

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

  it('puts the camera target at the centre, not the origin', () => {
    const at = { x: 120, y: -40, z: 0 };
    const p = project(at, { ...cam, target: at }, 800, 600);
    expect(p.x).toBeCloseTo(400);
    expect(p.y).toBeCloseTo(300);
  });

  it('translates before rotating, so the target holds the centre at any angle', () => {
    // Rotating first and translating after would swing the focused node around
    // the middle of the screen as the graph spins — the node you are standing on
    // would be the one thing that refuses to stay still.
    const at = { x: 120, y: -40, z: 60 };
    const turned = project(at, { ...cam, target: at, yaw: 1.1, pitch: 0.4 }, 800, 600);
    expect(turned.x).toBeCloseTo(400);
    expect(turned.y).toBeCloseTo(300);
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

describe('neighboursOf', () => {
  it('reports what a typed link claims, and prefers a claim to a mention', () => {
    const claims: Edge[] = [
      { source: 'feedback_rule', target: 'project_a', relation: null },
      { source: 'project_a', target: 'feedback_rule', relation: 'governs' },
    ];
    // Both memories link each other; only one of them says anything. The claim
    // is the half worth reporting.
    expect(neighboursOf(claims, 'project_a')).toEqual([
      { name: 'feedback_rule', direction: 'both', relation: 'governs' },
    ]);
  });

  it('lists one hop with the direction each link was written in', () => {
    expect(neighboursOf(EDGES, 'feedback_rule')).toEqual([
      { name: 'project_a', direction: 'in', relation: null },
      { name: 'project_b', direction: 'out', relation: null },
    ]);
  });

  it('reports a pair that cite each other once, as mutual', () => {
    const mutual: Edge[] = [
      { source: 'project_a', target: 'feedback_rule' },
      { source: 'feedback_rule', target: 'project_a' },
    ];
    expect(neighboursOf(mutual, 'project_a')).toEqual([
      { name: 'feedback_rule', direction: 'both', relation: null },
    ]);
  });

  it('ignores a memory that links to itself', () => {
    expect(neighboursOf([{ source: 'project_a', target: 'project_a' }], 'project_a')).toEqual([]);
  });

  it('orders by name, so a walk sees the same list twice', () => {
    const shuffled = [...EDGES].reverse();
    expect(neighboursOf(shuffled, 'feedback_rule')).toEqual(neighboursOf(EDGES, 'feedback_rule'));
  });

  it('gives nothing for a memory nothing links to', () => {
    expect(neighboursOf(EDGES, 'reference_lonely')).toEqual([]);
  });
});

describe('companionsOf', () => {
  const affinities = [
    { a: 'project_a', b: 'reference_x', npmi: 0.4, sessions: 5 },
    { a: 'feedback_rule', b: 'project_a', npmi: 0.9, sessions: 8 },
    { a: 'project_b', b: 'reference_y', npmi: 0.7, sessions: 4 },
  ];

  it('finds a pair from either end, since co-use has no direction', () => {
    // project_a is `a` in one row and `b` in the other. Reading only one field
    // would show the habit to one of the two memories and hide it from the
    // other, which is the bug this exists to prevent.
    expect(companionsOf(affinities, [], 'project_a').map((c) => c.name)).toEqual([
      'feedback_rule',
      'reference_x',
    ]);
  });

  it('marks whether the corpus already links the pair', () => {
    const linked: Edge[] = [{ source: 'feedback_rule', target: 'project_a', relation: 'governs' }];
    const found = companionsOf(affinities, linked, 'project_a');
    expect(found.map((c) => [c.name, c.linked])).toEqual([
      ['feedback_rule', true],
      ['reference_x', false],
    ]);
  });

  it('carries the support, not only the strength', () => {
    expect(companionsOf(affinities, [], 'reference_y')).toEqual([
      { name: 'project_b', npmi: 0.7, sessions: 4, linked: false },
    ]);
  });

  it('orders equal strengths by name, so the list does not reshuffle', () => {
    const tied = [
      { a: 'root', b: 'zeta', npmi: 0.5, sessions: 3 },
      { a: 'root', b: 'alpha', npmi: 0.5, sessions: 3 },
    ];
    expect(companionsOf(tied, [], 'root').map((c) => c.name)).toEqual(['alpha', 'zeta']);
  });

  it('reports no support rather than guessing when the miner gave none', () => {
    expect(companionsOf([{ a: 'root', b: 'other', npmi: 0.5 }], [], 'root')[0].sessions).toBe(0);
  });

  it('ignores a pair of one memory with itself', () => {
    expect(companionsOf([{ a: 'root', b: 'root', npmi: 1 }], [], 'root')).toEqual([]);
  });

  it('gives nothing for a memory nothing was ever used alongside', () => {
    expect(companionsOf(affinities, [], 'project_lonely')).toEqual([]);
  });
});

describe('clusterLevels', () => {
  /**
   * Two triangles joined by a single link. Any honest community detector has to
   * find exactly this: the two triangles, and the join as a join rather than as
   * a third group.
   */
  const BARBELL: Edge[] = [
    { source: 'a1', target: 'a2' },
    { source: 'a2', target: 'a3' },
    { source: 'a3', target: 'a1' },
    { source: 'b1', target: 'b2' },
    { source: 'b2', target: 'b3' },
    { source: 'b3', target: 'b1' },
    { source: 'a1', target: 'b1' },
  ];
  const BARBELL_NAMES = ['a1', 'a2', 'a3', 'b1', 'b2', 'b3'];

  it('finds the groups the links actually form', () => {
    const [finest] = clusterLevels(BARBELL_NAMES, BARBELL);
    const of = new Map<string, number>();
    finest.forEach((c, i) => c.members.forEach((m) => of.set(m, i)));
    expect(of.get('a1')).toBe(of.get('a2'));
    expect(of.get('a2')).toBe(of.get('a3'));
    expect(of.get('b1')).toBe(of.get('b2'));
    expect(of.get('a1')).not.toBe(of.get('b1'));
  });

  it('names each cluster after its most-connected member', () => {
    const [finest] = clusterLevels(BARBELL_NAMES, BARBELL);
    // a1 and b1 carry the joining link, so they outrank their triangle-mates.
    expect(finest.map((c) => c.core).sort()).toEqual(['a1', 'b1']);
  });

  it('clusters the same corpus identically twice', () => {
    // Louvain is normally randomised, which would make "which cluster is this
    // in?" a question with a different answer on every load.
    const a = clusterLevels(BARBELL_NAMES, BARBELL);
    const b = clusterLevels(BARBELL_NAMES, BARBELL);
    expect(a).toEqual(b);
  });

  it('does not care which way round a link was written', () => {
    const flipped = BARBELL.map((e) => ({ source: e.target, target: e.source }));
    expect(clusterLevels(BARBELL_NAMES, flipped)).toEqual(clusterLevels(BARBELL_NAMES, BARBELL));
  });

  it('gives coarser levels than it started with, and stops when nothing merges', () => {
    // Six nodes in three linked pairs: fine level = 3 clusters, then they merge.
    const chain: Edge[] = [
      { source: 'p1', target: 'p2' },
      { source: 'q1', target: 'q2' },
      { source: 'r1', target: 'r2' },
      { source: 'p2', target: 'q1' },
      { source: 'q2', target: 'r1' },
    ];
    const levels = clusterLevels(['p1', 'p2', 'q1', 'q2', 'r1', 'r2'], chain);
    expect(levels.length).toBeGreaterThan(0);
    for (let i = 1; i < levels.length; i++) {
      expect(levels[i].length).toBeLessThan(levels[i - 1].length);
    }
  });

  it('leaves a memory nothing links to in a cluster of its own', () => {
    const levels = clusterLevels([...BARBELL_NAMES, 'alone'], BARBELL);
    const solo = levels[0].find((c) => c.members.includes('alone'));
    expect(solo?.members).toEqual(['alone']);
  });

  it('accounts for every memory exactly once at every level', () => {
    // A partition that loses or duplicates a memory would make cluster sizes lie
    // and the legend's counts disagree with the corpus.
    for (const level of clusterLevels(BARBELL_NAMES, BARBELL)) {
      const all = level.flatMap((c) => c.members);
      expect(all.sort()).toEqual([...BARBELL_NAMES].sort());
    }
  });

  it('survives a corpus with no links at all', () => {
    expect(() => clusterLevels(['a', 'b'], [])).not.toThrow();
  });
});

describe('bridges', () => {
  it('ranks the memory whose links reach the most clusters first', () => {
    const of = new Map([
      ['hub', 0],
      ['a', 0],
      ['b', 1],
      ['c', 2],
      ['d', 3],
    ]);
    const edges: Edge[] = [
      { source: 'hub', target: 'a' },
      { source: 'hub', target: 'b' },
      { source: 'hub', target: 'c' },
      { source: 'b', target: 'd' },
    ];
    const found = bridges(['hub', 'a', 'b', 'c', 'd'], edges, of);
    expect(found[0]).toEqual({ name: 'hub', spans: 3 });
  });

  it('leaves out a memory whose links all stay inside one cluster', () => {
    // The distinction the picture cannot show: this memory can be busy — a hub
    // — and still join nothing to anything.
    const of = new Map([
      ['a', 0],
      ['b', 0],
      ['c', 0],
    ]);
    const edges: Edge[] = [
      { source: 'a', target: 'b' },
      { source: 'a', target: 'c' },
    ];
    expect(bridges(['a', 'b', 'c'], edges, of)).toEqual([]);
  });
});

describe('boundingRadius', () => {
  const spread = createLayout(NAMES, EDGES, SECTIONS);

  it('measures the whole graph from the origin by default', () => {
    expect(boundingRadius(spread)).toBeGreaterThan(1);
  });

  it('measures only the named subset', () => {
    const one = new Set([spread.nodes[0].name]);
    const centre = spread.nodes[0].pos;
    // The node is the centre of its own measurement, so the radius collapses to
    // the floor of 1 rather than reaching out to the rest of the corpus.
    expect(boundingRadius(spread, centre, one)).toBe(1);
    expect(boundingRadius(spread, centre)).toBeGreaterThan(1);
  });
});

describe('frameFor', () => {
  const spread = createLayout(NAMES, EDGES, SECTIONS);

  it('looks at the whole corpus from the origin when nothing is focused', () => {
    const framing = frameFor(spread, null, null, 412, 620);
    expect(framing.target).toEqual({ x: 0, y: 0, z: 0 });
    // The floor applies here too: a four-memory corpus is smaller than
    // MIN_FRAME_RADIUS, and framing it exactly would fill the canvas with four
    // enormous dots. Written through the floor rather than around it, because
    // the corpus this view actually serves is far past it.
    const radius = Math.max(MIN_FRAME_RADIUS, boundingRadius(spread));
    expect(framing.zoom).toBeCloseTo(fitZoom(radius, 412, 620));
  });

  it('centres on the focused memory itself', () => {
    const framing = frameFor(spread, 'feedback_rule', null, 412, 620);
    const i = spread.index.get('feedback_rule')!;
    expect(framing.target).toEqual({ ...spread.nodes[i].pos });
  });

  it('does not alias the node position it is centred on', () => {
    // Aliasing would make the camera track a node that every step moves,
    // silently skipping the easing the caller is about to apply.
    const framing = frameFor(spread, 'feedback_rule', null, 412, 620);
    const before = { ...framing.target };
    stepLayout(spread);
    expect(framing.target).toEqual(before);
  });

  it('frames a neighbourhood closer than the whole corpus', () => {
    for (let i = 0; i < 200; i++) stepLayout(spread);
    const near = neighbourhood(EDGES, NAME_LIST, 'project_a', 1);
    const whole = frameFor(spread, null, null, 412, 620);
    const focused = frameFor(spread, 'project_a', near, 412, 620);
    expect(focused.zoom).toBeGreaterThanOrEqual(whole.zoom);
  });

  it('refuses to zoom past the floor for a memory that links to nothing', () => {
    // A lone node has a neighbourhood radius of zero; without the floor the fit
    // would be hundreds of pixels per world unit and the reader would arrive at
    // one dot filling the canvas with nothing else in sight.
    const alone = new Set(['reference_lonely']);
    const framing = frameFor(spread, 'reference_lonely', alone, 412, 620);
    expect(framing.zoom).toBeCloseTo(fitZoom(MIN_FRAME_RADIUS, 412, 620));
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

describe('planLabels', () => {
  /** A fixed-width stand-in for canvas text measurement. */
  const measure = (text: string): number => text.length * 7;

  const candidate = (
    name: string,
    x: number,
    y: number,
    degree: number,
    pinned = false,
  ): LabelCandidate => ({ name, x, y, radius: 4, degree, pinned });

  it('never draws more than the budget', () => {
    // The bug this replaced used a degree cutoff, so a growing corpus labelled
    // ever more nodes: ~25 collided into unreadable stacks on the live data.
    const many = Array.from({ length: 40 }, (_, i) =>
      candidate(`node_${i}`, 20, i * 40, 100 - i),
    );

    const plan = planLabels(many, measure, 1000);

    expect(plan.drawn.length).toBeLessThanOrEqual(LABEL_BUDGET);
    expect(plan.overBudget).toBe(30);
  });

  it('labels the highest-degree nodes', () => {
    const nodes = [
      candidate('quiet', 20, 10, 1),
      candidate('hub', 20, 60, 99),
      candidate('middling', 20, 110, 50),
    ];

    const plan = planLabels(nodes, measure, 1000, 2);

    expect(plan.drawn.map((l) => l.name)).toEqual(['hub', 'middling']);
  });

  it('moves a label that would overprint one already placed', () => {
    // Same y, overlapping x. Dropping it was the old behaviour and it threw away
    // half the budget on the live corpus, because the highest-degree nodes are
    // exactly the ones clustered together.
    const nodes = [candidate('first', 20, 100, 10), candidate('second', 30, 100, 9)];

    const plan = planLabels(nodes, measure, 1000);

    expect(plan.drawn.map((l) => l.name)).toEqual(['first', 'second']);
    expect(plan.collided).toBe(0);
    // The first keeps its node's own line; the second steps off it.
    expect(plan.drawn[0].y).toBe(100);
    expect(plan.drawn[1].y).not.toBe(100);
  });

  it('still counts a label with nowhere left to go', () => {
    // Three labels stacked on one point exhaust the line above and below too.
    const nodes = [
      candidate('one', 20, 100, 10),
      candidate('two', 22, 100, 9),
      candidate('three', 24, 100, 8),
      candidate('four', 26, 100, 7),
    ];

    const plan = planLabels(nodes, measure, 1000);

    expect(plan.drawn.length).toBe(3);
    expect(plan.collided).toBe(1);
  });

  it('keeps both labels when they are far enough apart', () => {
    const nodes = [candidate('first', 20, 100, 10), candidate('second', 20, 300, 9)];

    expect(planLabels(nodes, measure, 1000).drawn.length).toBe(2);
  });

  it('flips a label to the left rather than let it leave the canvas', () => {
    // The other half of the original bug: names ran off the right edge mid-word.
    const nodes = [candidate('a_long_memory_name', 380, 100, 10)];

    const plan = planLabels(nodes, measure, 412);

    expect(plan.drawn.length).toBe(1);
    expect(plan.drawn[0].flipped).toBe(true);
    expect(plan.drawn[0].x).toBeLessThan(380);
  });

  it('drops a label that fits on neither side', () => {
    // A narrow canvas and a long name: flipping puts it off the left edge, so
    // there is nowhere legible to put it and it is counted rather than clipped.
    const nodes = [candidate('an_extremely_long_memory_name_here', 30, 100, 10)];

    const plan = planLabels(nodes, measure, 200);

    expect(plan.drawn.length).toBe(0);
    expect(plan.offCanvas).toBe(1);
  });

  it('always labels a pinned node, even past the budget', () => {
    // The hovered or selected node is what the reader is asking about.
    const many = Array.from({ length: 40 }, (_, i) =>
      candidate(`node_${i}`, 20, i * 40, 100 - i),
    );
    const asked = candidate('the_one_hovered', 20, 5000, 0, true);

    const plan = planLabels([...many, asked], measure, 1000);

    expect(plan.drawn.map((l) => l.name)).toContain('the_one_hovered');
  });

  it('places a pinned node first, so it keeps the best position', () => {
    // Both get drawn now — the hub steps to another line — but the node the
    // reader is pointing at is the one that keeps its own line.
    const nodes = [
      candidate('hub', 20, 100, 99),
      candidate('asked_about', 25, 100, 0, true),
    ];

    const plan = planLabels(nodes, measure, 1000);

    expect(plan.drawn[0].name).toBe('asked_about');
    expect(plan.drawn[0].y).toBe(100);
    expect(plan.drawn.map((l) => l.name)).toContain('hub');
  });

  it('reports nothing drawn for no candidates', () => {
    const plan = planLabels([], measure, 1000);

    expect(plan.drawn).toEqual([]);
    expect(plan.collided).toBe(0);
    expect(plan.offCanvas).toBe(0);
    expect(plan.overBudget).toBe(0);
  });
});

describe('panDelta', () => {
  const cam = { yaw: 0, pitch: 0, distance: 900, zoom: 1, target: { x: 0, y: 0, z: 0 } };

  it('moves the world under the cursor by exactly the drag', () => {
    // The property that matters: after panning, the point that was under the
    // finger is under the finger still. Anything else feels like the graph
    // slipping.
    const at = { x: 40, y: -25, z: 0 };
    const before = project(at, cam, 800, 600);
    const d = panDelta(30, 12, cam);
    const after = project(at, { ...cam, target: { x: -d.x, y: -d.y, z: -d.z } }, 800, 600);
    expect(after.x - before.x).toBeCloseTo(30);
    expect(after.y - before.y).toBeCloseTo(12);
  });

  it('holds the drag true when the camera is turned', () => {
    // Undoing the rotation is the whole job. With yaw and pitch applied, a naive
    // implementation that shifted the target along the world axes would send the
    // graph off at an angle to the finger.
    //
    // Measured on the target itself, because that is the plane the promise is
    // made about — see the parallax case below for what happens off it.
    const turned = { ...cam, yaw: 0.9, pitch: -0.4, zoom: 0.5 };
    const at = { x: 0, y: 0, z: 0 };
    const before = project(at, turned, 800, 600);
    const d = panDelta(-22, 17, turned);
    const after = project(at, { ...turned, target: { x: -d.x, y: -d.y, z: -d.z } }, 800, 600);
    expect(after.x - before.x).toBeCloseTo(-22);
    expect(after.y - before.y).toBeCloseTo(17);
  });

  it('lets nearer nodes outrun the drag, and further ones lag it', () => {
    // Not a defect: the drag is exact on the target's plane and everything else
    // moves by its own perspective factor. Pinned because the tempting "fix" —
    // dividing by each node's depth — would make the graph shear as it panned.
    const nearer = { x: 0, y: 0, z: -300 };
    const further = { x: 0, y: 0, z: 300 };
    const d = panDelta(40, 0, cam);
    const moved = { ...cam, target: { x: -d.x, y: -d.y, z: -d.z } };
    const dNear = project(nearer, moved, 800, 600).x - project(nearer, cam, 800, 600).x;
    const dFar = project(further, moved, 800, 600).x - project(further, cam, 800, 600).x;
    expect(dNear).toBeGreaterThan(40);
    expect(dFar).toBeLessThan(40);
  });

  it('moves further in world units when zoomed out', () => {
    const near = panDelta(100, 0, { ...cam, zoom: 2 });
    const far = panDelta(100, 0, { ...cam, zoom: 0.5 });
    expect(Math.abs(far.x)).toBeGreaterThan(Math.abs(near.x));
  });

  it('survives a zoom of zero rather than returning infinities', () => {
    const d = panDelta(10, 10, { ...cam, zoom: 0 });
    expect(Number.isFinite(d.x + d.y + d.z)).toBe(true);
  });
});

describe('affinities', () => {
  /** Two memories in different groups, with no link between them. */
  const APART: LayoutInput[] = [
    { name: 'a1', group: 'A' },
    { name: 'a2', group: 'A' },
    { name: 'b1', group: 'B' },
    { name: 'b2', group: 'B' },
  ];
  const settle = (affinities: { a: string; b: string; npmi: number }[]) => {
    const layout = createLayout(APART, [], ['A', 'B'], affinities);
    for (let i = 0; i < 600; i++) stepLayout(layout);
    const at = (n: string) => layout.nodes[layout.index.get(n) ?? -1].pos;
    return distance(at('a1'), at('b1'));
  };

  it('pulls two memories together that the corpus never linked', () => {
    // The whole point: they are in different regions and neither cites the
    // other, but the work keeps using them together.
    const without = settle([]);
    const with_ = settle([{ a: 'a1', b: 'b1', npmi: 0.9 }]);
    expect(with_).toBeLessThan(without);
  });

  it('pulls harder the stronger the evidence', () => {
    const weak = settle([{ a: 'a1', b: 'b1', npmi: 0.2 }]);
    const strong = settle([{ a: 'a1', b: 'b1', npmi: 0.95 }]);
    expect(strong).toBeLessThan(weak);
  });

  it('never pushes, whatever the artefact claims', () => {
    // A negative score would mean "these avoid each other", which is not
    // something thirteen sessions can support. Clamped, not trusted.
    const apart = settle([]);
    const negative = settle([{ a: 'a1', b: 'b1', npmi: -5 }]);
    expect(negative).toBeCloseTo(apart, 3);
  });

  it('leaves the stated structure the stronger of the two', () => {
    // A linked pair must still end up closer than a merely co-used pair of the
    // same nominal strength, or the corpus stops being the skeleton.
    const linked = createLayout(APART, [{ source: 'a1', target: 'b1' }], ['A', 'B']);
    const affine = createLayout(APART, [], ['A', 'B'], [{ a: 'a1', b: 'b1', npmi: 1 }]);
    for (let i = 0; i < 600; i++) {
      stepLayout(linked);
      stepLayout(affine);
    }
    const gap = (l: typeof linked) =>
      distance(l.nodes[l.index.get('a1') ?? -1].pos, l.nodes[l.index.get('b1') ?? -1].pos);
    expect(gap(linked)).toBeLessThan(gap(affine));
  });

  it('drops an affinity naming a memory the graph does not have', () => {
    const layout = createLayout(APART, [], ['A', 'B'], [{ a: 'a1', b: 'ghost', npmi: 1 }]);
    expect(layout.soft).toHaveLength(0);
  });
});
