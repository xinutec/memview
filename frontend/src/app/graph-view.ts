import {
  Component,
  DestroyRef,
  ElementRef,
  afterNextRender,
  computed,
  inject,
  signal,
  viewChild,
} from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatSlideToggleModule } from '@angular/material/slide-toggle';
import { MatSliderModule } from '@angular/material/slider';
import { RouterLink } from '@angular/router';

import {
  Camera,
  LabelPlan,
  Layout,
  LinkDirection,
  SETTLED,
  UNSECTIONED_COLOUR,
  createLayout,
  frameFor,
  neighbourhood,
  neighboursOf,
  planLabels,
  project,
  sectionColour,
  stepLayout,
} from './graph-layout';
import { MemviewApi } from './memview-api';
import { GraphData, GraphNode } from './models';

/** Below this drag distance a pointer gesture counts as a click, not a rotate. */
const CLICK_SLOP = 4;
/** Zoom bounds, in pixels per world unit at the origin plane. */
const MIN_ZOOM = 0.05;
const MAX_ZOOM = 20;
/** Padding between the framed graph and the canvas edge, as a fraction. */
const FIT_MARGIN = 1.15;
/**
 * Fraction of the remaining distance the camera closes each frame.
 *
 * A step has to be *watchable*: jumping the camera to the next memory tells the
 * reader where they arrived but not where it was, so the graph stops being a
 * map and becomes a slideshow. At 60fps this settles in about a fifth of a
 * second — long enough to follow, short enough not to feel like waiting.
 */
const CAMERA_EASE = 0.14;
/** World units, and a zoom ratio, below which the camera counts as arrived. */
const TARGET_EPSILON = 0.4;
const ZOOM_EPSILON = 0.002;
/** How many jump-box matches to offer at once. */
const JUMP_LIMIT = 8;

interface LegendRow {
  key: string | null;
  label: string;
  colour: string;
  count: number;
}

/** A memory offered as somewhere to walk to. */
interface MemoryRow {
  name: string;
  description: string;
  colour: string;
  /** Already somewhere on the trail — where walking here would rewind to. */
  visited: boolean;
}

/**
 * A memory one hop from where the reader is standing.
 *
 * Separate from a plain row because only a neighbour has a direction. A search
 * hit is not a link, and giving it a nominal one would put an arrow beside
 * every result claiming a relationship the corpus never recorded.
 */
interface NeighbourRow extends MemoryRow {
  direction: LinkDirection;
}

interface Placed {
  node: GraphNode;
  x: number;
  y: number;
  depth: number;
  radius: number;
  colour: string;
  lit: boolean;
}

/**
 * The corpus as a 3D link graph you can turn, fly through and walk.
 *
 * Canvas 2D with a hand-rolled projection rather than WebGL: ~320 nodes and
 * ~700 edges cost nothing to draw, text labels stay trivial, and the bundle
 * gains no dependency (the page must render over the VPN with no third-party
 * fetch). See graph-layout.ts for the maths.
 *
 * Walking is the point, and the picture alone cannot carry it. At corpus scale
 * a memory is a four-pixel dot among three hundred and forty, only ten of which
 * can be labelled at once, so "click the neighbour you want next" is not an
 * instruction a reader can follow. The trail, the neighbour list and the jump
 * box are how you actually move; the canvas shows you where that movement has
 * taken you.
 */
@Component({
  selector: 'app-graph-view',
  templateUrl: './graph-view.html',
  styleUrl: './graph-view.scss',
  imports: [
    FormsModule,
    RouterLink,
    MatButtonModule,
    MatFormFieldModule,
    MatIconModule,
    MatInputModule,
    MatProgressBarModule,
    MatSliderModule,
    MatSlideToggleModule,
  ],
})
export class GraphView {
  private api = inject(MemviewApi);
  private host = inject<ElementRef<HTMLElement>>(ElementRef);
  private canvasRef = viewChild.required<ElementRef<HTMLCanvasElement>>('canvas');

  readonly data = signal<GraphData | null>(null);
  /**
   * The walk so far, oldest first; the last entry is where the reader stands.
   *
   * The trail is the selection, rather than a history kept beside it. A graph
   * walk with no way back makes every wrong turn a restart, and keeping the two
   * as separate state is how they drift apart.
   */
  readonly trail = signal<readonly string[]>([]);
  readonly hovered = signal<GraphNode | null>(null);
  /** Hops of the selected memory's neighbourhood to keep lit. */
  readonly focusDepth = signal(1);
  readonly spin = signal(true);
  /** Hide everything outside the neighbourhood, rather than dimming it. */
  readonly isolate = signal(true);
  readonly hiddenSections = signal<ReadonlySet<string | null>>(new Set());
  readonly jump = signal('');

  private readonly byName = computed(() => {
    const graph = this.data();
    return new Map((graph?.nodes ?? []).map((n) => [n.name, n]));
  });

  readonly selected = computed<GraphNode | null>(() => {
    const name = this.trail().at(-1);
    return name === undefined ? null : (this.byName().get(name) ?? null);
  });

  readonly legend = computed<LegendRow[]>(() => {
    const graph = this.data();
    if (!graph) return [];
    const counts = new Map<string | null, number>();
    for (const node of graph.nodes) {
      counts.set(node.section, (counts.get(node.section) ?? 0) + 1);
    }
    const rows: LegendRow[] = graph.sections.map((title, i) => ({
      key: title,
      label: title,
      colour: sectionColour(i),
      count: counts.get(title) ?? 0,
    }));
    const loose = counts.get(null) ?? 0;
    if (loose > 0) {
      rows.push({
        key: null,
        label: 'indexed under no heading',
        colour: UNSECTIONED_COLOUR,
        count: loose,
      });
    }
    return rows;
  });

  /** Names to keep lit: the selected memory's neighbourhood, or all of them. */
  private readonly lit = computed<ReadonlySet<string> | null>(() => {
    const graph = this.data();
    const root = this.selected();
    if (!graph || !root) return null;
    return neighbourhood(
      graph.edges,
      graph.nodes.map((n) => n.name),
      root.name,
      this.focusDepth(),
    );
  });

  readonly litCount = computed(() => this.lit()?.size ?? 0);

  /** Where the reader can go from here, and which way each link was written. */
  readonly neighbours = computed<NeighbourRow[]>(() => {
    const graph = this.data();
    const here = this.selected();
    if (!graph || !here) return [];
    const walked = new Set(this.trail());
    return neighboursOf(graph.edges, here.name).map((n) => {
      const node = this.byName().get(n.name);
      return {
        name: n.name,
        direction: n.direction,
        // A link can name a memory that was never written — the API reports
        // those so the gap is visible rather than quietly dropped.
        description: node?.description ?? 'not written yet',
        colour: node ? this.colourFor(node) : UNSECTIONED_COLOUR,
        visited: walked.has(n.name),
      };
    });
  });

  /** Memories matching the jump box, by name or description. */
  readonly jumpHits = computed<MemoryRow[]>(() => {
    const needle = this.jump().trim().toLowerCase();
    const graph = this.data();
    if (!graph || needle.length < 2) return [];
    const walked = new Set(this.trail());
    return graph.nodes
      .filter(
        (n) =>
          n.name.toLowerCase().includes(needle) || n.description.toLowerCase().includes(needle),
      )
      // A name match is what the reader almost always meant; a description match
      // is the fallback that finds a memory whose slug you cannot remember.
      .sort((a, b) => {
        const an = a.name.toLowerCase().includes(needle) ? 0 : 1;
        const bn = b.name.toLowerCase().includes(needle) ? 0 : 1;
        return an - bn || a.name.localeCompare(b.name);
      })
      .slice(0, JUMP_LIMIT)
      .map((n) => ({
        name: n.name,
        description: n.description,
        colour: this.colourFor(n),
        visited: walked.has(n.name),
      }));
  });

  private camera: Camera = {
    yaw: 0.6,
    pitch: -0.25,
    distance: 900,
    zoom: 1,
    target: { x: 0, y: 0, z: 0 },
  };
  private layout: Layout | null = null;
  private ctx: CanvasRenderingContext2D | null = null;
  private placed: Placed[] = [];
  private colours = new Map<string, string>();
  private width = 0;

  /** What the last frame's labelling decided, including its rejections. */
  private labelPlan: LabelPlan | null = null;
  private height = 0;
  private running = false;
  private dirty = false;
  /** Whether the camera has framed the graph at all yet. */
  private fitted = false;
  /** Once the reader zooms by hand, stop re-framing under them. */
  private userZoomed = false;
  private dragging = false;
  private dragMoved = 0;
  private lastPointer: { x: number; y: number } | null = null;
  private theme = { text: '#000', edge: 'rgba(0,0,0,0.15)', halo: '#fff' };

  constructor() {
    const destroyRef = inject(DestroyRef);

    this.api.graph().subscribe((graph) => {
      this.colours = new Map(graph.sections.map((s, i) => [s, sectionColour(i)]));
      this.layout = createLayout(
        graph.nodes.map((n) => ({ name: n.name, section: n.section })),
        graph.edges,
        graph.sections,
      );
      this.fitted = false;
      this.data.set(graph);
      this.ensureLoop();
    });

    afterNextRender(() => {
      const canvas = this.canvasRef().nativeElement;
      this.ctx = canvas.getContext('2d');
      this.readTheme();
      const resize = new ResizeObserver(() => this.resize());
      resize.observe(canvas);
      this.resize();

      // A canvas holds pixels, not a stylesheet: nothing repaints it when the OS
      // theme flips, so the chrome colours must be re-resolved by hand.
      const scheme = window.matchMedia('(prefers-color-scheme: dark)');
      const onScheme = (): void => {
        this.readTheme();
        this.requestDraw();
      };
      scheme.addEventListener('change', onScheme);

      destroyRef.onDestroy(() => {
        resize.disconnect();
        scheme.removeEventListener('change', onScheme);
        this.running = false;
      });
    });
  }

  /**
   * Resolve the Material system tokens the chrome uses.
   *
   * `getComputedStyle(el).getPropertyValue('--mat-sys-…')` yields
   * `light-dark(#…, #…)`, which canvas cannot parse — and assigning an
   * unparseable `fillStyle` fails SILENTLY, keeping the previous colour (black
   * on a fresh context, invisible in dark mode only). Setting the token on a
   * real `color` property first makes the style engine collapse it to a used
   * value. The probe must be in the document for that to happen.
   */
  private readTheme(): void {
    const host = this.host.nativeElement;
    const probe = document.createElement('span');
    probe.style.display = 'none';
    host.appendChild(probe);
    const resolve = (token: string): string => {
      probe.style.color = `var(${token})`;
      return getComputedStyle(probe).color;
    };
    this.theme = {
      text: resolve('--mat-sys-on-surface'),
      edge: resolve('--mat-sys-outline-variant'),
      halo: resolve('--mat-sys-surface'),
    };
    probe.remove();
  }

  private resize(): void {
    const canvas = this.canvasRef().nativeElement;
    const rect = canvas.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    this.width = rect.width;
    this.height = rect.height;
    canvas.width = Math.max(1, Math.round(rect.width * dpr));
    canvas.height = Math.max(1, Math.round(rect.height * dpr));
    this.ctx?.setTransform(dpr, 0, 0, dpr, 0, 0);
    this.requestDraw();
  }

  private requestDraw(): void {
    this.dirty = true;
    this.ensureLoop();
  }

  /** Run frames only while something is moving; idle costs nothing. */
  private ensureLoop(): void {
    if (this.running) return;
    this.running = true;
    requestAnimationFrame(this.tick);
  }

  private tick = (): void => {
    if (!this.running) return;
    const layout = this.layout;
    if (!layout) {
      this.running = false;
      return;
    }
    let moving = false;
    if (layout.alpha > SETTLED) {
      stepLayout(layout);
      moving = true;
    }
    if (this.frame()) moving = true;
    // Spin is for reading structure, and it stops once the reader is reading
    // something specific: labels sliding out from under the memory you just
    // walked to are worse than a still picture.
    if (this.spin() && !this.dragging && !this.selected()) {
      this.camera.yaw += 0.0016;
      moving = true;
    }
    this.draw();
    if (moving || this.dirty) {
      this.dirty = false;
      requestAnimationFrame(this.tick);
    } else {
      this.running = false;
    }
  };

  /**
   * Move the camera toward what it should be looking at. Returns whether it is
   * still travelling.
   *
   * The first framing snaps, every later one eases. Snapping the first is what
   * stops the load reading as a bug: the camera starts at zoom 1, the settled
   * corpus needs about 0.54, and easing that gap on the first frame shows a
   * clipped close-up that then pulls back — which is exactly the "starts zoomed
   * in, zooms out a few seconds later" this view used to do.
   */
  private frame(): boolean {
    const layout = this.layout;
    if (!layout || this.width === 0) return false;
    const goal = frameFor(
      layout,
      this.selected()?.name ?? null,
      this.lit(),
      this.width,
      this.height,
      FIT_MARGIN,
    );
    const cam = this.camera;

    if (!this.fitted) {
      this.fitted = true;
      cam.target = goal.target;
      cam.zoom = goal.zoom;
      return true;
    }

    const dx = goal.target.x - cam.target.x;
    const dy = goal.target.y - cam.target.y;
    const dz = goal.target.z - cam.target.z;
    cam.target = {
      x: cam.target.x + dx * CAMERA_EASE,
      y: cam.target.y + dy * CAMERA_EASE,
      z: cam.target.z + dz * CAMERA_EASE,
    };
    let travelling = Math.hypot(dx, dy, dz) > TARGET_EPSILON;

    // A hand zoom is left alone — but only the zoom. Re-centring on the memory
    // the reader just walked to is the whole gesture, and refusing to do it
    // because they had scrolled earlier would strand them looking at the
    // previous one.
    if (!this.userZoomed) {
      const ratio = goal.zoom / cam.zoom;
      cam.zoom += (goal.zoom - cam.zoom) * CAMERA_EASE;
      if (Math.abs(ratio - 1) > ZOOM_EPSILON) travelling = true;
    }
    return travelling;
  }

  private colourFor(node: GraphNode): string {
    if (node.section === null) return UNSECTIONED_COLOUR;
    return this.colours.get(node.section) ?? UNSECTIONED_COLOUR;
  }

  private draw(): void {
    const ctx = this.ctx;
    const layout = this.layout;
    const graph = this.data();
    if (!ctx || !layout || !graph || this.width === 0) return;

    ctx.clearRect(0, 0, this.width, this.height);
    const hidden = this.hiddenSections();
    const lit = this.lit();
    const isolate = lit !== null && this.isolate();
    const selected = this.selected();
    const hovered = this.hovered();

    const placed: Placed[] = [];
    const screen = new Map<string, Placed>();
    for (let i = 0; i < graph.nodes.length; i++) {
      const node = graph.nodes[i];
      if (hidden.has(node.section)) continue;
      const isLit = lit === null || lit.has(node.name);
      // Isolating is not the same as dimming. The camera flies in close on a
      // walk, so the memories that are merely *near* the one being read are
      // drawn large — as unexplained blobs behind the thing you asked for.
      if (isolate && !isLit) continue;
      const p = project(layout.nodes[i].pos, this.camera, this.width, this.height);
      const degree = node.in_degree + node.out_degree;
      // Weighted toward degree, not body length: how connected a memory is says
      // more about its place in the graph than how long it is, and sizing by
      // bytes made a 97 KB roadmap outrank a load-bearing one-line rule.
      const radius = (2 + Math.log10(1 + node.size / 400) * 0.9 + Math.sqrt(degree) * 1.4) * p.scale;
      const entry: Placed = {
        node,
        x: p.x,
        y: p.y,
        depth: p.depth,
        radius: Math.max(1.2, radius),
        colour: this.colourFor(node),
        lit: isLit,
      };
      placed.push(entry);
      screen.set(node.name, entry);
    }
    // Painter's algorithm: far first, so near nodes occlude rather than blend.
    placed.sort((a, b) => b.depth - a.depth);
    this.placed = placed;

    ctx.lineWidth = 1;
    for (const edge of graph.edges) {
      const a = screen.get(edge.source);
      const b = screen.get(edge.target);
      if (!a || !b) continue;
      const bothLit = a.lit && b.lit;
      if (lit !== null && !bothLit) continue;
      ctx.globalAlpha = lit === null ? 0.22 : 0.55;
      ctx.strokeStyle = this.theme.edge;
      ctx.beginPath();
      ctx.moveTo(a.x, a.y);
      ctx.lineTo(b.x, b.y);
      ctx.stroke();
    }

    for (const entry of placed) {
      // Fog: distant nodes fade, which is most of what reads as depth on a flat
      // screen once the picture stops moving. Measured against the CURRENT
      // camera distance, not a constant — a fixed 900 stopped varying with zoom
      // and the picture read as a flat scatter.
      const fog = Math.max(0.12, Math.min(1, this.camera.distance / entry.depth));
      ctx.globalAlpha = entry.lit ? fog : fog * 0.12;
      ctx.fillStyle = entry.colour;
      ctx.beginPath();
      ctx.arc(entry.x, entry.y, entry.radius, 0, Math.PI * 2);
      ctx.fill();
      if (entry.node === selected || entry.node === hovered) {
        ctx.globalAlpha = 1;
        ctx.strokeStyle = this.theme.text;
        ctx.lineWidth = 2;
        ctx.stroke();
        ctx.lineWidth = 1;
      }
    }

    ctx.globalAlpha = 1;
    ctx.font = '11px system-ui, sans-serif';
    ctx.textBaseline = 'middle';
    this.drawLabels(ctx, placed, selected, hovered);
  }

  /**
   * Label the landmarks, nearest first, skipping any label that would collide
   * with one already drawn or run off the canvas.
   *
   * Both rules exist because the naive version was the single worst thing in the
   * picture: every node above a degree cutoff got a label, so ~25 long
   * snake_case names overprinted each other and several ran off the right edge
   * mid-word. A label you can't read is worse than no label — it also hides the
   * node it belongs to.
   */
  private drawLabels(
    ctx: CanvasRenderingContext2D,
    placed: Placed[],
    selected: GraphNode | null,
    hovered: GraphNode | null,
  ): void {
    const plan = planLabels(
      placed
        .filter((e) => e.lit || e.node === selected || e.node === hovered)
        .map((e) => ({
          name: e.node.name,
          x: e.x,
          y: e.y,
          radius: e.radius,
          degree: e.node.in_degree + e.node.out_degree,
          pinned: e.node === selected || e.node === hovered,
        })),
      (text) => ctx.measureText(text).width,
      this.width,
    );
    this.labelPlan = plan;

    for (const label of plan.drawn) {
      // Halo behind the text so a label crossing a dense region stays readable.
      ctx.lineWidth = 3;
      ctx.strokeStyle = this.theme.halo;
      ctx.strokeText(label.name, label.x, label.y);
      ctx.lineWidth = 1;
      ctx.fillStyle = this.theme.text;
      ctx.fillText(label.name, label.x, label.y);
    }
  }

  /** Nearest drawn node under a screen point, front-most first. */
  private hit(x: number, y: number): GraphNode | null {
    for (let i = this.placed.length - 1; i >= 0; i--) {
      const entry = this.placed[i];
      const dx = entry.x - x;
      const dy = entry.y - y;
      const reach = Math.max(entry.radius, 6);
      if (dx * dx + dy * dy <= reach * reach) return entry.node;
    }
    return null;
  }

  onPointerDown(event: PointerEvent): void {
    this.dragging = true;
    this.dragMoved = 0;
    this.lastPointer = { x: event.clientX, y: event.clientY };
    if (event.target instanceof Element) event.target.setPointerCapture(event.pointerId);
  }

  onPointerMove(event: PointerEvent): void {
    const canvas = this.canvasRef().nativeElement;
    const rect = canvas.getBoundingClientRect();
    if (this.dragging && this.lastPointer) {
      const dx = event.clientX - this.lastPointer.x;
      const dy = event.clientY - this.lastPointer.y;
      this.dragMoved += Math.abs(dx) + Math.abs(dy);
      this.camera.yaw += dx * 0.006;
      this.camera.pitch = Math.max(
        -Math.PI / 2,
        Math.min(Math.PI / 2, this.camera.pitch + dy * 0.006),
      );
      this.lastPointer = { x: event.clientX, y: event.clientY };
      this.requestDraw();
      return;
    }
    const found = this.hit(event.clientX - rect.left, event.clientY - rect.top);
    if (found !== this.hovered()) {
      this.hovered.set(found);
      this.requestDraw();
    }
  }

  onPointerUp(event: PointerEvent): void {
    const canvas = this.canvasRef().nativeElement;
    const rect = canvas.getBoundingClientRect();
    const wasDrag = this.dragMoved > CLICK_SLOP;
    this.dragging = false;
    this.lastPointer = null;
    if (wasDrag) return;
    const found = this.hit(event.clientX - rect.left, event.clientY - rect.top);
    // Clicking empty space clears the walk — the way out.
    if (found) this.walkTo(found.name);
    else this.clearSelection();
  }

  onWheel(event: WheelEvent): void {
    event.preventDefault();
    // Scrolling down (positive deltaY) pulls back, so zoom shrinks.
    const factor = Math.exp(-event.deltaY * 0.001);
    this.camera.zoom = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, this.camera.zoom * factor));
    this.userZoomed = true;
    this.requestDraw();
  }

  /**
   * Take one step of the walk.
   *
   * Arriving somewhere already on the trail rewinds to it rather than appending.
   * Two memories that cite each other are one tap apart, so an appending trail
   * would grow an unbounded there-and-back-again of the same two names and the
   * back button would replay it one step at a time.
   */
  walkTo(name: string): void {
    if (!this.byName().has(name)) return;
    const trail = this.trail();
    const at = trail.indexOf(name);
    this.trail.set(at === -1 ? [...trail, name] : trail.slice(0, at + 1));
    // A deliberate move re-earns the right to frame the picture: the reader
    // asked to go somewhere, and leaving them at a hand-set zoom that no longer
    // shows it would be obeying the letter of "don't re-frame under them".
    this.userZoomed = false;
    this.jump.set('');
    this.ensureLoop();
  }

  back(): void {
    this.trail.set(this.trail().slice(0, -1));
    this.userZoomed = false;
    this.ensureLoop();
  }

  toggleSection(key: string | null): void {
    const next = new Set<string | null>(this.hiddenSections());
    if (!next.delete(key)) next.add(key);
    this.hiddenSections.set(next);
    this.requestDraw();
  }

  isHidden(key: string | null): boolean {
    return this.hiddenSections().has(key);
  }

  setSpin(on: boolean): void {
    this.spin.set(on);
    if (on) this.ensureLoop();
  }

  setIsolate(on: boolean): void {
    this.isolate.set(on);
    this.ensureLoop();
  }

  setDepth(depth: number): void {
    this.focusDepth.set(depth);
    this.ensureLoop();
  }

  clearSelection(): void {
    this.trail.set([]);
    this.userZoomed = false;
    this.ensureLoop();
  }

  /** The glyph for how a link between two memories was written. */
  arrow(direction: LinkDirection): string {
    if (direction === 'out') return 'arrow_forward';
    if (direction === 'in') return 'arrow_back';
    return 'sync_alt';
  }

  /** Re-heat the simulation — a settled layout can sit in a local minimum. */
  reheat(): void {
    if (!this.layout) return;
    this.layout.alpha = 1;
    this.ensureLoop();
  }
}
