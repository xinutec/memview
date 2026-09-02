/**
 * Where a picture sits in the frame it is being looked at in.
 *
 * Separated from the sheet because this is arithmetic, and arithmetic about
 * gestures is exactly the kind that is wrong in ways nobody can see by reading
 * it: an anchor that drifts by a few pixels per pinch looks like a slippery
 * screen rather than like a bug. Everything here is a pure function of a
 * [[View]] and a gesture, so the sheet is left holding pointers and this is left
 * holding the model.
 *
 * The picture is laid out **fitted** — as large as it goes with its whole self
 * showing — and that rect is the base every number here is relative to. A
 * [[View]] is what CSS then does to it: `translate(x, y) scale(scale)` about the
 * centre. So `FIT` is the identity, and `scale` is magnification over *fitted*
 * rather than over the picture's own pixels.
 */

/** A magnification, and where the picture has been moved to, in pixels. */
export interface View {
  readonly scale: number;
  readonly x: number;
  readonly y: number;
}

export interface Size {
  readonly width: number;
  readonly height: number;
}

/** A point relative to the centre of the frame, which is where a `transform`
 *  measures from. */
export interface Point {
  readonly x: number;
  readonly y: number;
}

/** The whole picture, centred: what a viewer opens on and returns to. */
export const FIT: View = { scale: 1, x: 0, y: 0 };

/**
 * As close as it will go.
 *
 * ⚠ **Magnification over the fitted size, not over the file.** A 600px render
 * fitted to a 412px phone is already shrunk, so eight times fitted is nearer
 * five times its own pixels — enough to see whether a wall came out straight,
 * and past the point where anything is left to see.
 */
export const CLOSEST = 8;

/** Where one tap takes it, when it is not already in. */
export const CLOSER = 2.5;

/**
 * Clamped, and never to negative zero.
 *
 * ⚠ **`-0` is what a clamp to a slack of nothing produces**, and it is equal to
 * `0` everywhere except the two places that matter: `Object.is`, which is what
 * every equality assertion and every "has this changed?" check is built on. A
 * centred picture would then differ from a centred picture. CSS cannot tell them
 * apart, so nothing here is being made prettier — it is being made comparable.
 */
const clamp = (value: number, low: number, high: number) => {
  const held = Math.min(Math.max(value, low), high);
  return held === 0 ? 0 : held;
};

/**
 * The size a picture is drawn at when it is fitted, which CSS decides and this
 * has to agree with.
 *
 * ⚠ **Never enlarged**, because `max-width: 100%` only ever shrinks: a 200px
 * thumbnail in a 412px frame is drawn at 200px, and a model that assumed it
 * filled the frame would let it be dragged around a gap that is not there.
 */
export function fittedIn(picture: Size, frame: Size): Size {
  if (picture.width <= 0 || picture.height <= 0) return { width: 0, height: 0 };
  const scale = Math.min(frame.width / picture.width, frame.height / picture.height, 1);
  return { width: picture.width * scale, height: picture.height * scale };
}

/**
 * The same view, with the picture kept inside its frame.
 *
 * ⚠ **The slack is what the picture has that the frame does not**, halved
 * because the transform measures from the centre: at fitted size there is none
 * in at least one axis, so that axis stays centred however hard it is dragged.
 * Without this the picture can be flung off the screen and there is no gesture
 * that says "come back" — the only way out is closing the sheet.
 */
export function bounded(view: View, frame: Size, base: Size): View {
  const scale = clamp(view.scale, 1, CLOSEST);
  const slackX = Math.max(0, (base.width * scale - frame.width) / 2);
  const slackY = Math.max(0, (base.height * scale - frame.height) / 2);
  return {
    scale,
    x: clamp(view.x, -slackX, slackX),
    y: clamp(view.y, -slackY, slackY),
  };
}

/**
 * Zoom by a factor about a point, leaving what is under that point where it is.
 *
 * ⚠ **This is the whole feel of a pinch.** Scaling about the centre instead is a
 * line of code shorter and wrong in a way that is hard to name and impossible
 * not to notice: the thing you were looking at slides away while you zoom
 * towards it, so you chase it with the other hand.
 *
 * The anchor is kept by moving the picture so the point's distance from it grows
 * with the scale. The factor is recomputed after clamping rather than before, so
 * a pinch that runs into [[CLOSEST]] stops magnifying without also sliding.
 */
export function scaledAbout(view: View, at: Point, by: number, frame: Size, base: Size): View {
  const scale = clamp(view.scale * by, 1, CLOSEST);
  const factor = scale / view.scale;
  return bounded(
    {
      scale,
      x: at.x - (at.x - view.x) * factor,
      y: at.y - (at.y - view.y) * factor,
    },
    frame,
    base,
  );
}

/** Move the picture under a finger, as far as it will go. */
export function moved(view: View, by: Point, frame: Size, base: Size): View {
  return bounded({ ...view, x: view.x + by.x, y: view.y + by.y }, frame, base);
}

/**
 * One tap: in about the point, or back to the whole picture.
 *
 * ⚠ **Out is [[FIT]] exactly, not the reverse of the way in.** Undoing a zoom by
 * dividing leaves the offset it was dragged to, so the picture comes back
 * off-centre and a second tap is needed to see all of it — which reads as a
 * control that half worked.
 */
export function toggled(view: View, at: Point, frame: Size, base: Size): View {
  if (view.scale > 1) return FIT;
  return scaledAbout(FIT, at, CLOSER, frame, base);
}

/** What a pinch did, from where the two fingers were and where they are. */
export function pinched(
  was: readonly [Point, Point],
  now: readonly [Point, Point],
): { at: Point; by: number } {
  const apart = (pair: readonly [Point, Point]) =>
    Math.hypot(pair[0].x - pair[1].x, pair[0].y - pair[1].y);
  const middle = (pair: readonly [Point, Point]) => ({
    x: (pair[0].x + pair[1].x) / 2,
    y: (pair[0].y + pair[1].y) / 2,
  });
  const before = apart(was);
  // Two fingers in the same place are not a pinch yet, and dividing by that
  // distance is how a viewer jumps to CLOSEST on the first frame of a gesture.
  return { at: middle(now), by: before > 0 ? apart(now) / before : 1 };
}
