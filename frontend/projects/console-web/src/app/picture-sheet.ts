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
 * How far a finger may wander before the gesture stops being a tap.
 *
 * A thumb on glass never holds still, and at zero every tap was a one-pixel drag
 * that then refused to toggle.
 */
const SLIP = 8;

/** How much wheel it takes to double the magnification. `Math.exp` rather than a
 *  step, so a trackpad's stream of small deltas is smooth and a mouse's notches
 *  still move it. */
const WHEEL = 300;

/** Which picture is being looked at: the address as the session wrote it. */
export interface Looking {
  readonly url: string;
}

/**
 * A picture a session pointed at, opened over the conversation.
 *
 * ⚠ **A lightbox here, where the sent-picture path deliberately has none.** A
 * picture sent from the phone is already in the transcript with the words about
 * it around it, so it opens in place — covering those words would take away the
 * reason for looking. This one is not in the transcript at all: there is nothing
 * to expand and nothing to cover, and what it usually holds is a render of a
 * room that is unreadable at a quarter of a phone screen.
 *
 * ⚠ **Back closes the viewer and only the viewer.** That is [[Dismiss]]'s
 * history entry, the same as every other sheet — without it, a back gesture
 * would close this *and* leave the conversation behind it, which is what
 * Material's `closeOnNavigation` does on its own.
 *
 * The bytes come through [[ConsoleApi.elsewhere]] rather than from an `<img
 * src>`, because the failures here are ordinary — the session's render server
 * outlives its links by minutes — and an `<img>` that fails says nothing about
 * why.
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
   * The fingers currently on the picture, by the id the browser gives each.
   *
   * ⚠ **A `Map` and not two fields.** A third finger landing mid-pinch, or a
   * pointer whose `up` never arrives because the sheet closed under it, are both
   * ordinary — and the version of this that tracked "the first" and "the second"
   * pointer left a stale one behind after either, so the next single-finger drag
   * was read as a pinch against a finger that was no longer there.
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
    // ⚠ **A blob URL is a reference the document holds until it is revoked**, and
    // these are megabytes. Opening six renders in a conversation without this
    // keeps all six in the tab for as long as it lives.
    const at = this.at();
    if (at) URL.revokeObjectURL(at);
  }

  protected close(): void {
    this.sheet.dismiss();
  }

  /**
   * The frame's size and what the picture is drawn at inside it.
   *
   * ⚠ **Read from the elements every time, not remembered.** The phone rotates,
   * the browser's chrome comes and goes, and the picture's own size is not known
   * until it has loaded — a measurement taken once is wrong after any of those,
   * and what it produces is a picture that cannot be dragged to its own edge.
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
   * A page point, as the transform measures: from the middle of the frame.
   *
   * Everything `zoom.ts` is given is relative to that centre, because that is
   * where `transform-origin` puts it. A gesture's anchor arrives in page
   * coordinates and has to be moved into those before it means anything.
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
    // ⚠ **Captured, so the gesture survives leaving the element.** A drag that
    // reaches the edge of a magnified picture otherwise stops getting `move`
    // events, and the finger is still down: the next `up` lands somewhere else
    // and the pointer is never cleared.
    this.frame()?.nativeElement.setPointerCapture(event.pointerId);
    this.fingers.set(event.pointerId, { x: event.clientX, y: event.clientY });
    this.travelled = false;
  }

  /**
   * A finger moved: pinch if there is another one down, pan if not.
   *
   * ⚠ **The two positions of the pair come from the map, either side of this one
   * update.** Only one finger moves per event — the browser sends a `move` for
   * each — so "where they were" is the map before the write and "where they are"
   * is the map after it. Keeping a separate copy of the pair instead lost the
   * first increment of every pinch, because the copy could only be taken once a
   * finger had already moved: a spread from 60px to 240px, which is four times,
   * came out as 1.6.
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

    if (after.length >= 2) {
      // The first two, so a third finger joining mid-gesture changes nothing —
      // a Map keeps its insertion order, including when a key is written again.
      const gesture = pinched([before[0], before[1]], [after[0], after[1]]);
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
   * The wheel, for the same picture at a desk.
   *
   * The console is read on a phone and driven from a browser on this Mac, and a
   * viewer that can only be worked with two fingers is unusable in the second.
   * `preventDefault` because the alternative is the page scrolling behind it.
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
   * A tap, or Enter on the focused picture: in about that point, or back out.
   *
   * ⚠ **Not after a drag.** A pan ends with a `click` on the element it started
   * on, so without this every time the picture was moved it also jumped to
   * fitted — the gesture undoing itself at the moment it finished.
   */
  protected tapped(event: MouseEvent): void {
    if (this.travelled) return;
    const measures = this.measures();
    if (!measures) return;
    // A keyboard `click` reports the element's corner rather than a point on the
    // picture; the centre is what "look closer" means with no place to look at.
    const at =
      event.detail === 0 ? { x: 0, y: 0 } : this.at_point({ x: event.clientX, y: event.clientY });
    this.view.update((view) => toggled(view, at, measures.frame, measures.base));
  }

  /**
   * Why the picture did not arrive.
   *
   * ⚠ **Asking for a `Blob` means the failure arrives as one too.** The console
   * answers a failed fetch with a sentence — "it answered 404 Not Found", "what
   * came back is not a PNG… it begins `<!DOCTYPE html>`" — and with
   * `responseType: 'blob'` that sentence is a `Blob` on `err.error`, where
   * [[reason]] finds no string and falls back to "the runner answered 502". So
   * it is read out here, and [[reason]] answers everything else.
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
