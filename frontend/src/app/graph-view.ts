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
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { MatButtonModule } from '@angular/material/button';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatSlideToggleModule } from '@angular/material/slide-toggle';
import { MatSliderModule } from '@angular/material/slider';
import { ActivatedRoute, Router, RouterLink } from '@angular/router';

import {
  Camera,
  Cluster,
  LabelPlan,
  Layout,
  LinkDirection,
  SETTLED,
  UNSECTIONED_COLOUR,
  bridges,
  clusterLevels,
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
/**
 * How many clusters make a readable map, used to pick the opening grain.
 *
 * The corpus supports several groupings — on the live corpus 103, then 34, then
 * 23 — and none of them is the right one. This is a legibility budget, not a
 * claim about the structure: a dozen or two named regions is what a reader can
 * hold at once, and the ladder is exposed so they can go finer or coarser.
 */
const READABLE_CLUSTERS = 20;

/** A cluster as the legend shows it: what it is called, how big, what colour. */
interface ClusterRow {
  /** Position in the sorted legend — also the hue index. */
  index: number;
  core: string;
  members: readonly string[];
  colour: string;
  /** Nothing links to it and it links to nothing; it is a cluster of one. */
  alone: boolean;
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
 * instruction a reader can follow. The clusters and the neighbour list are how
 * you move; the canvas shows you where that movement has taken you.
 */
@Component({
  selector: 'app-graph-view',
  templateUrl: './graph-view.html',
  styleUrl: './graph-view.scss',
  imports: [
    RouterLink,
    MatButtonModule,
    MatIconModule,
    MatProgressBarModule,
    MatSliderModule,
    MatSlideToggleModule,
  ],
})
export class GraphView {
  private api = inject(MemviewApi);
  private router = inject(Router);
  private route = inject(ActivatedRoute);
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
  /** Which cluster is being read as a whole, by legend index. */
  readonly focusedCluster = signal<number | null>(null);

  /**
   * The cluster ladder, coarsening from left to right.
   *
   * Derived, not stored: the clusters are a pure function of the links, so
   * holding them as state could only ever let them disagree with the graph they
   * came from.
   */
  readonly levels = computed<readonly Cluster[][]>(() => {
    const graph = this.data();
    if (!graph) return [];
    return clusterLevels(
      graph.nodes.map((n) => n.name),
      graph.edges,
    );
  });

  /**
   * The rung nearest a readable number of regions.
   *
   * Opening at the finest grain would be a true reading of the corpus and a
   * useless map — on the live corpus that is 103 clusters.
   */
  private readonly defaultGrain = computed(() => {
    const levels = this.levels();
    let best = 0;
    levels.forEach((level, i) => {
      const closer =
        Math.abs(level.length - READABLE_CLUSTERS)
        < Math.abs(levels[best].length - READABLE_CLUSTERS);
      if (closer) best = i;
    });
    return best;
  });

  /** null until the reader picks a grain of their own. */
  private readonly pickedGrain = signal<number | null>(null);
  readonly grain = computed(() => this.pickedGrain() ?? this.defaultGrain());

  private readonly byName = computed(() => {
    const graph = this.data();
    return new Map((graph?.nodes ?? []).map((n) => [n.name, n]));
  });

  readonly selected = computed<GraphNode | null>(() => {
    const name = this.trail().at(-1);
    return name === undefined ? null : (this.byName().get(name) ?? null);
  });

  /**
   * The clusters at the current grain, biggest first.
   *
   * Sorted by size so the largest regions get the most separated hues and a
   * stable place in the legend; ties break on the core's name so the order does
   * not depend on which memory the API happened to list first.
   */
  readonly clusters = computed<ClusterRow[]>(() => {
    const level = this.levels()[this.grain()] ?? [];
    return [...level]
      .sort((a, b) => b.members.length - a.members.length || a.core.localeCompare(b.core))
      .map((c, index) => ({
        index,
        core: c.core,
        members: c.members,
        // A cluster of one gets no hue. Colouring it would imply it belongs
        // somewhere, and the whole point of showing it is that it does not.
        colour: c.members.length > 1 ? sectionColour(index) : UNSECTIONED_COLOUR,
        alone: c.members.length === 1,
      }));
  });

  /** Which cluster each memory landed in, by legend index. */
  private readonly clusterOf = computed(() => {
    const of = new Map<string, number>();
    for (const row of this.clusters()) for (const m of row.members) of.set(m, row.index);
    return of;
  });

  /**
   * How many clusters each memory's links reach into.
   *
   * The corpus's load-bearing joins, and not the same thing as its hubs — on
   * the live corpus the top bridge is a *rule*, feedback_verify_assumptions,
   * which reaches 10 of 23 clusters. A picture where both hubs and bridges are
   * just large dots cannot tell you that.
   */
  readonly spans = computed(() => {
    const graph = this.data();
    if (!graph) return new Map<string, number>();
    const found = bridges(
      graph.nodes.map((n) => n.name),
      graph.edges,
      this.clusterOf(),
    );
    return new Map(found.map((b) => [b.name, b.spans]));
  });

  /** Memories in no cluster but their own — nothing links to them, or from. */
  readonly stranded = computed(() => this.clusters().filter((c) => c.alone));

  /**
   * Names to keep lit: a whole cluster, or the walk's neighbourhood, or all.
   *
   * A cluster reading wins over a walk because it is the coarser question — you
   * ask "what is in here" before "where does this one go" — and answering both
   * at once would light a union that is neither.
   */
  private readonly lit = computed<ReadonlySet<string> | null>(() => {
    const graph = this.data();
    if (!graph) return null;
    const cluster = this.focusedCluster();
    if (cluster !== null) {
      const row = this.clusters().find((c) => c.index === cluster);
      if (row) return new Set(row.members);
    }
    const root = this.selected();
    if (!root) return null;
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

  /**
   * A walk read from the URL that has not been applied yet.
   *
   * The graph arrives asynchronously, and a walk names memories that only exist
   * once it has. Applying the names against an empty corpus would silently drop
   * every one of them and land the reader on an unfocused graph with no hint
   * that their link had said otherwise.
   */
  private pendingWalk: readonly string[] | null = null;

  constructor() {
    const destroyRef = inject(DestroyRef);

    // The walk lives in ?walk= so a path through the corpus can be linked, and
    // so the browser's own back gesture — the one a phone reader will reach for
    // — undoes a step rather than leaving the graph entirely.
    this.route.queryParamMap.pipe(takeUntilDestroyed()).subscribe((params) => {
      const walk = (params.get('walk') ?? '').split(',').filter(Boolean);
      // Guard against the write below bouncing straight back in as a read.
      if (walk.join(',') === this.trail().join(',')) return;
      this.pendingWalk = walk;
      this.applyWalk();
    });

    this.api.graph().subscribe((graph) => {
      this.data.set(graph);
      this.rebuildLayout();
      this.applyWalk();
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
    // A focused cluster is framed on its core — the member it is named after —
    // so the region the reader picked from the legend arrives centred on the
    // memory whose name they read.
    const cluster = this.focusedCluster();
    const centre =
      cluster === null
        ? (this.selected()?.name ?? null)
        : (this.clusters().find((c) => c.index === cluster)?.core ?? null);
    const goal = frameFor(layout, centre, this.lit(), this.width, this.height, FIT_MARGIN);
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

  /**
   * A memory's colour is its cluster's, found in the links — not its curated
   * section. On the live corpus those two disagree for about half the corpus at
   * map scale, and the picture should show the structure that is actually there.
   */
  /**
   * Rebuild the layout so each cluster gets its own territory.
   *
   * A cluster of one is handed no group at all, so it anchors at the origin and
   * floats unattached in the middle. That is the honest picture of a memory
   * nothing links to — and on the live corpus it puts all twelve of them where
   * they cannot be missed, instead of scattering them into regions they have no
   * connection to.
   */
  private rebuildLayout(): void {
    const graph = this.data();
    if (!graph) return;
    const groupOf = new Map<string, string>();
    const groups: string[] = [];
    for (const row of this.clusters()) {
      if (row.alone) continue;
      groups.push(row.core);
      for (const member of row.members) groupOf.set(member, row.core);
    }
    this.layout = createLayout(
      graph.nodes.map((n) => ({ name: n.name, group: groupOf.get(n.name) ?? null })),
      graph.edges,
      groups,
    );
    this.fitted = false;
    this.ensureLoop();
  }

  private colourFor(node: GraphNode): string {
    const index = this.clusterOf().get(node.name);
    if (index === undefined) return UNSECTIONED_COLOUR;
    return this.clusters()[index]?.colour ?? UNSECTIONED_COLOUR;
  }

  private draw(): void {
    const ctx = this.ctx;
    const layout = this.layout;
    const graph = this.data();
    if (!ctx || !layout || !graph || this.width === 0) return;

    ctx.clearRect(0, 0, this.width, this.height);
    const lit = this.lit();
    const isolate = lit !== null && this.isolate();
    const selected = this.selected();
    const hovered = this.hovered();

    const placed: Placed[] = [];
    const screen = new Map<string, Placed>();
    for (let i = 0; i < graph.nodes.length; i++) {
      const node = graph.nodes[i];
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
  /** Take up a walk read from the URL, once there is a corpus to check it against. */
  private applyWalk(): void {
    const walk = this.pendingWalk;
    if (walk === null || this.data() === null) return;
    this.pendingWalk = null;
    // A link can outlive the memory it names: a walk is written down once and
    // the corpus is rewritten daily. Unknown names are dropped rather than
    // rejecting the whole walk, so an old link still lands you as close to
    // where it meant as the corpus still allows.
    this.setTrail(walk.filter((name) => this.byName().has(name)));
  }

  private setTrail(walk: readonly string[]): void {
    this.trail.set(walk);
    // Stepping onto a memory ends the cluster reading: the reader has stopped
    // asking what is in the region and started asking where this one goes.
    if (walk.length) this.focusedCluster.set(null);
    // A deliberate move re-earns the right to frame the picture: the reader
    // asked to go somewhere, and leaving them at a hand-set zoom that no longer
    // shows it would be obeying the letter of "don't re-frame under them".
    this.userZoomed = false;
    // Not awaited: the trail signal is already set, so the picture has moved —
    // the navigation only records where it moved to. It resolves false when a
    // faster second step supersedes it, which is the correct outcome and not
    // something to recover from.
    void this.router.navigate([], {
      relativeTo: this.route,
      // null, not '': an empty value would leave a bare `?walk=` on the URL of
      // a graph nobody is walking.
      queryParams: { walk: walk.length ? walk.join(',') : null },
      queryParamsHandling: 'merge',
    });
    this.ensureLoop();
  }

  walkTo(name: string): void {
    if (!this.byName().has(name)) return;
    const trail = this.trail();
    const at = trail.indexOf(name);
    this.setTrail(at === -1 ? [...trail, name] : trail.slice(0, at + 1));
  }

  back(): void {
    this.setTrail(this.trail().slice(0, -1));
  }

  /** Read a whole cluster: light its members and fly the camera to its core. */
  focusCluster(index: number): void {
    this.focusedCluster.set(this.focusedCluster() === index ? null : index);
    this.userZoomed = false;
    this.ensureLoop();
  }

  /**
   * Change the grain, which rebuilds the picture.
   *
   * The clusters are the layout's anchors, so a coarser reading is a genuinely
   * different arrangement of the same memories rather than a recolouring — the
   * simulation has to run again for the regions to re-form.
   */
  setGrain(level: number): void {
    if (level === this.grain()) return;
    this.pickedGrain.set(level);
    this.focusedCluster.set(null);
    this.rebuildLayout();
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
    this.setTrail([]);
  }

  /** What cluster a memory landed in, for the focus card. */
  clusterName(name: string): string {
    const index = this.clusterOf().get(name);
    const row = index === undefined ? undefined : this.clusters()[index];
    if (!row || row.alone) return 'in no cluster';
    return row.core === name ? `core of its cluster (${row.members.length})` : `with ${row.core}`;
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
