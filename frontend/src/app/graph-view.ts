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
import { MatButtonModule } from '@angular/material/button';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatSlideToggleModule } from '@angular/material/slide-toggle';
import { MatSliderModule } from '@angular/material/slider';
import { RouterLink } from '@angular/router';

import {
  Camera,
  LabelPlan,
  Layout,
  SETTLED,
  UNSECTIONED_COLOUR,
  boundingRadius,
  createLayout,
  fitZoom,
  neighbourhood,
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

interface LegendRow {
  key: string | null;
  label: string;
  colour: string;
  count: number;
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
  private host = inject<ElementRef<HTMLElement>>(ElementRef);
  private canvasRef = viewChild.required<ElementRef<HTMLCanvasElement>>('canvas');

  readonly data = signal<GraphData | null>(null);
  readonly selected = signal<GraphNode | null>(null);
  readonly hovered = signal<GraphNode | null>(null);
  /** Hops of the selected memory's neighbourhood to keep lit. */
  readonly focusDepth = signal(1);
  readonly spin = signal(true);
  readonly hiddenSections = signal<ReadonlySet<string | null>>(new Set());

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

  private camera: Camera = { yaw: 0.6, pitch: -0.25, distance: 900, zoom: 1 };
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
  /** Whether the camera has framed the settled layout yet. */
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
    // Re-frame for the new canvas size, unless the reader has chosen a zoom —
    // silently undoing their zoom on an orientation change would be worse.
    if (this.fitted && !this.userZoomed) this.fit();
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
    // Frame the graph on every frame it is still moving, not once when it stops.
    //
    // Fitting only on settle meant the camera sat at its initial zoom of 1 for
    // the whole simulation — 259 steps, about 4.3 seconds at 60fps — while the
    // graph expanded past the edges, and then snapped to the fitted zoom (0.537
    // on the live corpus) in a single frame. It read as the view being broken and
    // then correcting itself. Re-fitting continuously makes that same change a
    // gradual zoom-out that tracks the layout as it spreads.
    //
    // Cheap: boundingRadius is one pass over ~350 nodes, against a step that
    // already does the pairwise force loop.
    if (!this.userZoomed && (moving || !this.fitted)) {
      this.fit();
    }
    if (this.spin() && !this.dragging) {
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
    const selected = this.selected();
    const hovered = this.hovered();

    const placed: Placed[] = [];
    const screen = new Map<string, Placed>();
    for (let i = 0; i < graph.nodes.length; i++) {
      const node = graph.nodes[i];
      if (hidden.has(node.section)) continue;
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
        lit: lit === null || lit.has(node.name),
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
    // Clicking empty space clears the focus — the way out of a walk.
    this.selected.set(found);
    this.requestDraw();
  }

  onWheel(event: WheelEvent): void {
    event.preventDefault();
    // Scrolling down (positive deltaY) pulls back, so zoom shrinks.
    const factor = Math.exp(-event.deltaY * 0.001);
    this.camera.zoom = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, this.camera.zoom * factor));
    this.userZoomed = true;
    this.requestDraw();
  }

  /** Frame the settled graph in the current canvas. */
  private fit(): void {
    const layout = this.layout;
    if (!layout || this.width === 0) return;
    this.fitted = true;
    this.camera.zoom = fitZoom(boundingRadius(layout), this.width, this.height, FIT_MARGIN);
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

  setDepth(depth: number): void {
    this.focusDepth.set(depth);
    this.requestDraw();
  }

  clearSelection(): void {
    this.selected.set(null);
    this.requestDraw();
  }

  /** Re-heat the simulation — a settled layout can sit in a local minimum. */
  reheat(): void {
    if (!this.layout) return;
    this.layout.alpha = 1;
    this.ensureLoop();
  }
}
