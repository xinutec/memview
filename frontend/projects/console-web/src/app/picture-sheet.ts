import {
  Component,
  ElementRef,
  OnDestroy,
  computed,
  inject,
  signal,
  viewChild,
} from '@angular/core';
import { MAT_BOTTOM_SHEET_DATA, MatBottomSheetRef } from '@angular/material/bottom-sheet';
import { MatButtonModule } from '@angular/material/button';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';

import { ConsoleApi } from './console-api';
import { reason } from './errors';
import {
  FIT,
  fittedIn,
  moved,
  pinched,
  scaledAbout,
  toggled,
  type Point,
  type Size,
  type View,
} from './zoom';

/**
 * How far a finger may wander before the gesture stops being a tap: at zero
 * every tap was a one-pixel drag.
 */
const SLIP = 8;

/**
 * How much wheel it takes to double the magnification. `Math.exp` rather than a
 * step, so a trackpad's stream of small deltas is smooth.
 */
const WHEEL = 300;

/** Which picture is being looked at: the address as the session wrote it. */
export interface Looking {
  readonly url: string;
}

/**
 * A picture a session pointed at, opened over the conversation.
 *
 * A lightbox here, where a sent picture opens in place: that one has the words
 * about it around it, this one is not in the transcript at all and is usually a
 * render unreadable at a quarter of a phone screen. Back closes the viewer and
 * only the viewer — [[Dismiss]]'s history entry. The bytes come through
 * [[ConsoleApi.elsewhere]] because an `<img>` that fails says nothing about why.
 */
@Component({
  selector: 'app-picture-sheet',
  templateUrl: './picture-sheet.html',
  styleUrl: './picture-sheet.scss',
  imports: [MatButtonModule, MatIconModule, MatProgressBarModule],
})
export class PictureSheet implements OnDestroy {
  private readonly given = inject<Looking>(MAT_BOTTOM_SHEET_DATA);
  private api = inject(ConsoleApi);
  private sheet = inject(MatBottomSheetRef<PictureSheet>);

  /** The address as it was written, which is how a person tells renders apart. */
  protected readonly url = this.given.url;
  /** The blob URL the picture is drawn from, once it has arrived. */
  protected readonly at = signal<string | undefined>(undefined);
  /** Why there is no picture, in the console's own words. */
  protected readonly trouble = signal('');

  private readonly frame = viewChild<ElementRef<HTMLElement>>('frame');
  private readonly picture = viewChild<ElementRef<HTMLImageElement>>('picture');

  /** Where the picture is, as a magnification and an offset. See `zoom.ts`. */
  private readonly view = signal<View>(FIT);
  /** What the template puts on the `img`. */
  protected readonly drawn = computed(() => {
    const view = this.view();
    return `translate(${view.x}px, ${view.y}px) scale(${view.scale})`;
  });
  /** Whether it is magnified at all — the label and the cursor both change. */
  protected readonly close_up = computed(() => this.view().scale > 1);

  /**
   * The fingers currently on the picture, by the id the browser gives each. A
   * `Map`: a third finger mid-pinch, or a pointer whose `up` never arrives, are
   * ordinary, and two fields left a stale one behind.
   */
  private readonly fingers = new Map<number, Point>();
  /** Whether this gesture has moved far enough to be a drag rather than a tap. */
  private travelled = false;

  constructor() {
    this.api
      .elsewhere(this.given.url)
      .pipe(takeUntilDestroyed())
      .subscribe({
        next: (bytes) => this.at.set(URL.createObjectURL(bytes)),
        error: (err: unknown) => void this.explain(err),
      });
  }

  ngOnDestroy(): void {
    // A blob URL is a reference the document holds until it is revoked, and these
    // are megabytes.
    const at = this.at();
    if (at) URL.revokeObjectURL(at);
  }

  protected close(): void {
    this.sheet.dismiss();
  }

  /**
   * The frame's size and what the picture is drawn at inside it — read from the
   * elements every time: the phone rotates, the chrome comes and goes, and the
   * picture's own size is not known until it has loaded.
   */
  private measures(): { frame: Size; base: Size } | undefined {
    const frame = this.frame()?.nativeElement;
    const picture = this.picture()?.nativeElement;
    if (!frame || !picture?.naturalWidth) return undefined;
    const box = frame.getBoundingClientRect();
    const size = { width: box.width, height: box.height };
    return {
      frame: size,
      base: fittedIn({ width: picture.naturalWidth, height: picture.naturalHeight }, size),
    };
  }

  /**
   * A page point, as the transform measures: from the middle of the frame, which
   * is where `transform-origin` puts it.
   */
  private at_point(page: Point): Point {
    const box = this.frame()?.nativeElement.getBoundingClientRect();
    if (!box) return { x: 0, y: 0 };
    return { x: page.x - (box.left + box.width / 2), y: page.y - (box.top + box.height / 2) };
  }

  /** A picture that has arrived is fitted, whatever the last one was doing. */
  protected measured(): void {
    this.view.set(FIT);
  }

  protected took(event: PointerEvent): void {
    // Captured, so the gesture survives leaving the element: a drag reaching the
    // edge otherwise stops getting `move` events with the finger still down.
    this.frame()?.nativeElement.setPointerCapture(event.pointerId);
    this.fingers.set(event.pointerId, { x: event.clientX, y: event.clientY });
    this.travelled = false;
  }

  /**
   * A finger moved: pinch if there is another one down, pan if not. The two
   * positions of the pair come from the map either side of this update — only one
   * finger moves per event. A separate copy of the pair lost the first increment
   * of every pinch.
   */
  protected drew(event: PointerEvent): void {
    const was = this.fingers.get(event.pointerId);
    const measures = this.measures();
    if (!was || !measures) return;
    const now = { x: event.clientX, y: event.clientY };
    const before = [...this.fingers.values()];
    this.fingers.set(event.pointerId, now);
    const after = [...this.fingers.values()];
    if (Math.hypot(now.x - was.x, now.y - was.y) > SLIP) this.travelled = true;

    // The first two, so a third finger joining changes nothing — a Map keeps
    // insertion order, including when a key is written again, so `before` and
    // `after` are the same length and a pair on one is a pair on the other.
    const [wasFirst, wasSecond] = before;
    const [first, second] = after;
    if (wasFirst && wasSecond && first && second) {
      const gesture = pinched([wasFirst, wasSecond], [first, second]);
      this.view.update((view) =>
        scaledAbout(view, this.at_point(gesture.at), gesture.by, measures.frame, measures.base),
      );
      return;
    }

    this.view.update((view) =>
      moved(view, { x: now.x - was.x, y: now.y - was.y }, measures.frame, measures.base),
    );
  }

  protected let_go(event: PointerEvent): void {
    this.fingers.delete(event.pointerId);
  }

  /**
   * The wheel, for the same picture at a desk. `preventDefault`, or the page
   * scrolls behind it.
   */
  protected rolled(event: WheelEvent): void {
    const measures = this.measures();
    if (!measures) return;
    event.preventDefault();
    this.view.update((view) =>
      scaledAbout(
        view,
        this.at_point({ x: event.clientX, y: event.clientY }),
        Math.exp(-event.deltaY / WHEEL),
        measures.frame,
        measures.base,
      ),
    );
  }

  /**
   * A tap, or Enter on the focused picture: in about that point, or back out. Not
   * after a drag — a pan ends with a `click`, and the gesture would undo itself.
   */
  protected tapped(event: MouseEvent): void {
    if (this.travelled) return;
    const measures = this.measures();
    if (!measures) return;
    // A keyboard `click` reports the element's corner; the centre is what "look
    // closer" means with no place to look at.
    const at =
      event.detail === 0 ? { x: 0, y: 0 } : this.at_point({ x: event.clientX, y: event.clientY });
    this.view.update((view) => toggled(view, at, measures.frame, measures.base));
  }

  /**
   * Why the picture did not arrive. With `responseType: 'blob'` the console's
   * sentence arrives as a `Blob` on `err.error`, where [[reason]] finds no string;
   * so it is read out here.
   */
  private async explain(err: unknown): Promise<void> {
    const body: unknown = err && typeof err === 'object' ? Reflect.get(err, 'error') : undefined;
    if (body instanceof Blob) {
      const said = (await body.text()).trim();
      if (said) {
        this.trouble.set(said);
        return;
      }
    }
    this.trouble.set(reason(err));
  }
}
