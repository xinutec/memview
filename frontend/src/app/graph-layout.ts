/**
 * 3D force-directed layout, perspective projection and neighbourhood search for
 * the memory link graph.
 *
 * Deliberately dependency-free and separate from the component: at corpus scale
 * (~320 nodes, ~700 edges) a hand-rolled O(n²) solver is a few hundred lines
 * cheaper than a WebGL graph library and renders in plain canvas 2D, where text
 * labels are trivial. Keeping the maths here also makes it testable — a canvas
 * proves nothing under vitest, which has neither layout nor pixels.
 *
 * Everything is deterministic: the same corpus lays out identically twice, so a
 * bug report or a screenshot is reproducible. `Math.random` would forfeit that.
 */

export interface Vec3 {
  x: number;
  y: number;
  z: number;
}

export interface LayoutNode {
  name: string;
  pos: Vec3;
  vel: Vec3;
  /** This node's section anchor; null = unsectioned, anchored at the origin. */
  anchor: Vec3 | null;
}

export interface Layout {
  nodes: LayoutNode[];
  /** name → index into `nodes`. */
  index: Map<string, number>;
  /** Edges as index pairs — resolved once, so the hot loop never hashes. */
  pairs: [number, number][];
  /** Simulation temperature: 1 at rest-start, cooling to 0. */
  alpha: number;
}

/** What the layout needs to know about a memory: who it is, and where it belongs. */
export interface LayoutInput {
  name: string;
  /** MEMORY.md section, or null for memories the index files under no heading. */
  section: string | null;
}

export interface Edge {
  source: string;
  target: string;
}

export interface Camera {
  /** Rotation about the vertical axis, radians. */
  yaw: number;
  /** Rotation about the horizontal axis, radians. */
  pitch: number;
  /**
   * Eye distance from the origin, in world units. This sets how strongly
   * perspective bites (how much nearer things loom) — it is NOT the zoom.
   * Deriving scale from distance alone gives `distance / depth`, which is
   * exactly 1 at the origin plane whatever the distance: the picture then
   * renders one world unit per pixel forever, no fit is possible, and a graph
   * wider than the canvas is permanently clipped.
   */
  distance: number;
  /** Pixels per world unit at the origin plane — the actual zoom. */
  zoom: number;
}

export interface Projected {
  x: number;
  y: number;
  /** Distance from the eye — painter's-algorithm sort key and fog input. */
  depth: number;
  /** Perspective scale factor at this depth (1 = at the origin plane). */
  scale: number;
}

const REPULSION = 2400;
const SPRING = 0.045;
const REST_LENGTH = 26;
const DAMPING = 0.82;
const CENTERING = 0.004;
const COOLING = 0.985;
/**
 * Pull toward the node's section anchor. This is what stops the picture reading
 * as a random scatter: without it the ONLY forces are links, and since ~half the
 * corpus's links cross sections, the curated colours smear uniformly through one
 * ball. Kept well below SPRING so links still visibly drag a memory toward what
 * it cites — sections claim territory, they don't imprison.
 */
const SECTION_PULL = 0.022;
const ORIGIN: Vec3 = { x: 0, y: 0, z: 0 };
/** Below this the picture has stopped visibly moving, so the loop can idle. */
export const SETTLED = 0.02;
/** Guards the inverse-square term when two nodes land almost on top of another. */
const MIN_DISTANCE = 0.5;

/**
 * Deterministic, well-spread starting positions: the Fibonacci sphere, whose
 * golden-angle spiral avoids the clumping at the poles that naive spherical
 * coordinates produce. A good starting spread matters — force layouts fall into
 * local minima, and a clumped start reliably finds a bad one.
 */
function seedPosition(i: number, total: number, radius: number): Vec3 {
  const t = total > 1 ? i / (total - 1) : 0.5;
  const y = 1 - 2 * t;
  const ring = Math.sqrt(Math.max(0, 1 - y * y));
  const theta = Math.PI * (3 - Math.sqrt(5)) * i;
  return {
    x: Math.cos(theta) * ring * radius,
    y: y * radius,
    z: Math.sin(theta) * ring * radius,
  };
}

/**
 * Give every section a home on a sphere, in the order MEMORY.md lists them.
 *
 * Sections adjacent in the index land adjacent in space, so the picture keeps
 * the reading order of the curated taxonomy. Unsectioned memories anchor at the
 * origin — floating unattached in the middle is an honest depiction of a memory
 * the index files under no heading.
 */
function sectionAnchors(sections: readonly string[], radius: number): Map<string, Vec3> {
  const anchors = new Map<string, Vec3>();
  sections.forEach((section, i) => {
    anchors.set(section, seedPosition(i, sections.length, radius));
  });
  return anchors;
}

export function createLayout(
  inputs: readonly LayoutInput[],
  edges: readonly Edge[],
  sections: readonly string[] = [],
): Layout {
  const radius = 12 * Math.cbrt(Math.max(1, inputs.length));
  const anchors = sectionAnchors(sections, radius * 0.62);
  const index = new Map<string, number>();
  inputs.forEach((input, i) => index.set(input.name, i));
  const nodes: LayoutNode[] = inputs.map((input, i) => {
    const anchor = input.section === null ? null : (anchors.get(input.section) ?? null);
    const seed = seedPosition(i, inputs.length, radius);
    // Start inside the section's territory rather than anywhere on the sphere:
    // a force layout settles into whatever local minimum it starts near, so
    // seeding by section is most of what makes the sections hold together.
    const pos =
      anchor === null
        ? { x: seed.x * 0.35, y: seed.y * 0.35, z: seed.z * 0.35 }
        : {
            x: anchor.x + seed.x * 0.3,
            y: anchor.y + seed.y * 0.3,
            z: anchor.z + seed.z * 0.3,
          };
    return { name: input.name, pos, vel: { x: 0, y: 0, z: 0 }, anchor };
  });
  const pairs: [number, number][] = [];
  for (const edge of edges) {
    const a = index.get(edge.source);
    const b = index.get(edge.target);
    // An edge naming a node we don't have is dropped rather than crashing the
    // layout: the API shouldn't emit one, and if it ever does, the view degrades
    // instead of going blank.
    if (a !== undefined && b !== undefined && a !== b) pairs.push([a, b]);
  }
  return { nodes, index, pairs, alpha: 1 };
}

/**
 * One simulation tick: all-pairs repulsion, spring attraction along edges, a
 * weak pull to the origin so disconnected components can't drift away forever.
 * Mutates in place and cools `alpha`.
 */
export function stepLayout(layout: Layout): void {
  const { nodes, pairs } = layout;
  const n = nodes.length;
  for (const node of nodes) {
    node.vel.x *= DAMPING;
    node.vel.y *= DAMPING;
    node.vel.z *= DAMPING;
  }

  for (let i = 0; i < n; i++) {
    const a = nodes[i];
    for (let j = i + 1; j < n; j++) {
      const b = nodes[j];
      let dx = a.pos.x - b.pos.x;
      let dy = a.pos.y - b.pos.y;
      let dz = a.pos.z - b.pos.z;
      let d2 = dx * dx + dy * dy + dz * dz;
      if (d2 < MIN_DISTANCE) {
        // Coincident nodes have no direction to separate along; nudge them apart
        // along a fixed axis mix so the result stays deterministic.
        dx = (i % 3) - 1;
        dy = (j % 3) - 1;
        dz = ((i + j) % 3) - 1;
        d2 = Math.max(MIN_DISTANCE, dx * dx + dy * dy + dz * dz);
      }
      const d = Math.sqrt(d2);
      const force = REPULSION / d2 / d;
      a.vel.x += dx * force;
      a.vel.y += dy * force;
      a.vel.z += dz * force;
      b.vel.x -= dx * force;
      b.vel.y -= dy * force;
      b.vel.z -= dz * force;
    }
  }

  for (const [i, j] of pairs) {
    const a = nodes[i];
    const b = nodes[j];
    const dx = b.pos.x - a.pos.x;
    const dy = b.pos.y - a.pos.y;
    const dz = b.pos.z - a.pos.z;
    const d = Math.sqrt(dx * dx + dy * dy + dz * dz) || MIN_DISTANCE;
    const force = SPRING * (d - REST_LENGTH);
    const ux = (dx / d) * force;
    const uy = (dy / d) * force;
    const uz = (dz / d) * force;
    a.vel.x += ux;
    a.vel.y += uy;
    a.vel.z += uz;
    b.vel.x -= ux;
    b.vel.y -= uy;
    b.vel.z -= uz;
  }

  for (const node of nodes) {
    // An unsectioned node anchors at the origin, and at the SAME strength as a
    // sectioned one. Leaving it to the much weaker CENTERING term instead let
    // repulsion win and flung it to the outer shell — the opposite of the
    // "floating unattached in the middle" this is meant to depict.
    const anchor = node.anchor ?? ORIGIN;
    node.vel.x += (anchor.x - node.pos.x) * SECTION_PULL;
    node.vel.y += (anchor.y - node.pos.y) * SECTION_PULL;
    node.vel.z += (anchor.z - node.pos.z) * SECTION_PULL;
    node.vel.x -= node.pos.x * CENTERING;
    node.vel.y -= node.pos.y * CENTERING;
    node.vel.z -= node.pos.z * CENTERING;
    node.pos.x += node.vel.x * layout.alpha;
    node.pos.y += node.vel.y * layout.alpha;
    node.pos.z += node.vel.z * layout.alpha;
  }

  layout.alpha *= COOLING;
}

/** Distance from the origin to the furthest node — what the camera must frame. */
export function boundingRadius(layout: Layout): number {
  let max = 1;
  for (const node of layout.nodes) {
    const d = Math.hypot(node.pos.x, node.pos.y, node.pos.z);
    if (d > max) max = d;
  }
  return max;
}

/** World point → screen point. Yaw about Y, then pitch about X, then perspective. */
export function project(p: Vec3, cam: Camera, width: number, height: number): Projected {
  const cy = Math.cos(cam.yaw);
  const sy = Math.sin(cam.yaw);
  const cp = Math.cos(cam.pitch);
  const sp = Math.sin(cam.pitch);
  const x1 = p.x * cy - p.z * sy;
  const z1 = p.x * sy + p.z * cy;
  const y2 = p.y * cp - z1 * sp;
  const z2 = p.y * sp + z1 * cp;
  // Clamp so a node that swings behind the eye is pushed to the far plane rather
  // than projecting to a mirrored position in front of it.
  const depth = Math.max(1, z2 + cam.distance);
  const scale = (cam.zoom * cam.distance) / depth;
  return { x: width / 2 + x1 * scale, y: height / 2 + y2 * scale, depth, scale };
}

/**
 * The zoom that frames a graph of `radius` world units inside `width`×`height`,
 * leaving `margin` proportional padding so nodes don't sit against the edge.
 */
export function fitZoom(radius: number, width: number, height: number, margin = 1.15): number {
  return Math.min(width, height) / 2 / Math.max(1, radius * margin);
}

/**
 * Names within `depth` hops of `root`, following links in BOTH directions.
 *
 * Direction is deliberately ignored: "what is this connected to" doesn't depend
 * on which memory happened to write the link. This is what answers *which rules
 * govern this project* — the rules cite the projects about as often as the
 * reverse, so a directed walk would miss half of them.
 *
 * The returned set includes `root` itself (at hop 0). An unknown root yields an
 * empty set, not a set containing a name the graph doesn't have.
 */
export function neighbourhood(
  edges: readonly Edge[],
  names: readonly string[],
  root: string,
  depth: number,
): Set<string> {
  const found = new Set<string>();
  if (!names.includes(root)) return found;
  const adjacency = new Map<string, string[]>();
  const link = (a: string, b: string): void => {
    const list = adjacency.get(a);
    if (list) list.push(b);
    else adjacency.set(a, [b]);
  };
  for (const edge of edges) {
    link(edge.source, edge.target);
    link(edge.target, edge.source);
  }
  found.add(root);
  let frontier = [root];
  for (let hop = 0; hop < depth; hop++) {
    const next: string[] = [];
    for (const name of frontier) {
      for (const neighbour of adjacency.get(name) ?? []) {
        if (!found.has(neighbour)) {
          found.add(neighbour);
          next.push(neighbour);
        }
      }
    }
    if (next.length === 0) break;
    frontier = next;
  }
  return found;
}

/**
 * A distinct hue per section, spaced by the golden angle so that neighbouring
 * sections in the legend never get neighbouring hues.
 *
 * Returned as a literal `hsl()` string, NOT a Material system token: those
 * compute to `light-dark(…)`, which canvas cannot parse — and an unparseable
 * `fillStyle` assignment fails *silently*, leaving the previous colour. Data
 * colours are fixed here; only the chrome follows the theme.
 */
export function sectionColour(index: number): string {
  const hue = Math.round((index * 137.508) % 360);
  return `hsl(${hue} 72% 55%)`;
}

/** The colour for a memory with no `## section` — grey, and visibly not a hue. */
export const UNSECTIONED_COLOUR = 'hsl(0 0% 60%)';

/**
 * How many landmark labels to draw at most.
 *
 * A budget rather than a degree cutoff. Degree is a property of the corpus, not
 * of the picture, so as the corpus grows the same cutoff labels ever more nodes:
 * at degree >= 10 the live corpus drew ~25 labels that collided into unreadable
 * stacks. A fixed budget keeps the picture legible whatever the corpus does.
 */
export const LABEL_BUDGET = 10;

/** Half the line box a label occupies, for collision purposes. */
const LABEL_HALF_HEIGHT = 7;

/** Keep text this far inside the canvas edge. */
const EDGE_MARGIN = 4;

/** Gap between a node's circle and its text. */
const LABEL_GAP = 4;

/** A node that could be labelled, already projected to screen coordinates. */
export interface LabelCandidate {
  readonly name: string;
  readonly x: number;
  readonly y: number;
  readonly radius: number;
  readonly degree: number;
  /** Labelled whatever its degree — the node under the pointer, or the one
   *  being walked from. Those are what the reader is asking about. */
  readonly pinned: boolean;
}

export interface PlacedLabel {
  readonly name: string;
  readonly x: number;
  readonly y: number;
  readonly width: number;
  /** Drawn to the left of its node because the text would have run off. */
  readonly flipped: boolean;
}

/**
 * What labelling decided — including everything it decided against.
 *
 * The rejections are returned rather than discarded because they are the useful
 * signal: a picture that silently drops nine labels in ten looks the same as one
 * with nothing to say. `scripts/graph-report.mjs` reads these.
 */
export interface LabelPlan {
  readonly drawn: readonly PlacedLabel[];
  /** Dropped: the text ran off the canvas even after flipping to the left. */
  readonly offCanvas: number;
  /** Dropped: would have overprinted a label already placed. */
  readonly collided: number;
  /** Never considered — beyond the budget. */
  readonly overBudget: number;
}

/**
 * Choose which labels to draw and where.
 *
 * Pure, and separated from the canvas on purpose. This is the logic behind the
 * worst bug the graph view has had — ~25 long snake_case names overprinting each
 * other, several running off the right edge mid-word — and while it lived inside
 * the drawing routine it could not be tested at all, because exercising it meant
 * having a real `CanvasRenderingContext2D`.
 *
 * Text measurement stays with the caller as [measure]: only the canvas knows how
 * wide a string renders in the current font, and inventing an approximation here
 * would make the tests agree with a model of text rather than with text.
 */
export function planLabels(
  candidates: readonly LabelCandidate[],
  measure: (text: string) => number,
  canvasWidth: number,
  budget: number = LABEL_BUDGET,
): LabelPlan {
  // Pinned first, unconditionally — not merely "added if the budget was full",
  // which is what this did while it lived in the drawing routine. A pinned node
  // inside the budget was then placed in degree order, so a hub's label could
  // collide with it and drop the very node the reader was pointing at. The
  // comment there claimed these were labelled "whatever their degree"; they were
  // only guaranteed to be *considered*, which is not the same promise.
  const pinned = candidates.filter((c) => c.pinned);
  const ranked = candidates.filter((c) => !c.pinned).sort((a, b) => b.degree - a.degree);
  const considered = ranked.slice(0, budget);
  const queue = [...pinned, ...considered];

  const drawn: PlacedLabel[] = [];
  const boxes: { x: number; y: number; w: number; h: number }[] = [];
  let offCanvas = 0;
  let collided = 0;

  for (const entry of queue) {
    const width = measure(entry.name);
    const gap = entry.radius + LABEL_GAP;
    const flipped = entry.x + gap + width > canvasWidth - EDGE_MARGIN;
    const x = flipped ? entry.x - gap - width : entry.x + gap;
    if (x < EDGE_MARGIN) {
      offCanvas++;
      continue;
    }
    const box = { x, y: entry.y - LABEL_HALF_HEIGHT, w: width, h: LABEL_HALF_HEIGHT * 2 };
    const clash = boxes.some(
      (d) => box.x < d.x + d.w && box.x + box.w > d.x && box.y < d.y + d.h && box.y + box.h > d.y,
    );
    if (clash) {
      collided++;
      continue;
    }
    boxes.push(box);
    drawn.push({ name: entry.name, x, y: entry.y, width, flipped });
  }

  return {
    drawn,
    offCanvas,
    collided,
    overBudget: ranked.length - considered.length,
  };
}
