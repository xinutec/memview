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

export interface Edge {
  source: string;
  target: string;
}

export interface Camera {
  /** Rotation about the vertical axis, radians. */
  yaw: number;
  /** Rotation about the horizontal axis, radians. */
  pitch: number;
  /** Eye distance from the origin, in world units. */
  distance: number;
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
const CENTERING = 0.008;
const COOLING = 0.985;
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

export function createLayout(names: readonly string[], edges: readonly Edge[]): Layout {
  const radius = 12 * Math.cbrt(Math.max(1, names.length));
  const index = new Map<string, number>();
  names.forEach((name, i) => index.set(name, i));
  const nodes: LayoutNode[] = names.map((name, i) => ({
    name,
    pos: seedPosition(i, names.length, radius),
    vel: { x: 0, y: 0, z: 0 },
  }));
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
    node.vel.x -= node.pos.x * CENTERING;
    node.vel.y -= node.pos.y * CENTERING;
    node.vel.z -= node.pos.z * CENTERING;
    node.pos.x += node.vel.x * layout.alpha;
    node.pos.y += node.vel.y * layout.alpha;
    node.pos.z += node.vel.z * layout.alpha;
  }

  layout.alpha *= COOLING;
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
  const scale = cam.distance / depth;
  return { x: width / 2 + x1 * scale, y: height / 2 + y2 * scale, depth, scale };
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
