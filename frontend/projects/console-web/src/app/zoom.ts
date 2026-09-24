/**
 * Where a picture sits in the frame it is being looked at in.
 *
 * Separated from the sheet because arithmetic about gestures is wrong in ways
 * nobody can see by reading it: an anchor drifting a few pixels per pinch looks
 * like a slippery screen. Everything here is a pure function of a [[View]] and a
 * gesture.
 *
 * The picture is laid out fitted — as large as it goes with its whole self
 * showing — and a [[View]] is what CSS does to that: `translate(x, y)
 * scale(scale)` about the centre. `FIT` is the identity.
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
 * As close as it will go — magnification over the fitted size, not the file: a
 * 600px render fitted to a 412px phone is already shrunk.
 */
export const CLOSEST = 8;

/** Where one tap takes it, when it is not already in. */
export const CLOSER = 2.5;

/**
 * Clamped, and never to negative zero: `-0` differs from `0` under `Object.is`,
 * which every equality assertion and "has this changed?" check is built on.
 */
const clamp = (value: number, low: number, high: number) => {
  const held = Math.min(Math.max(value, low), high);
  return held === 0 ? 0 : held;
};

/**
 * The size a picture is drawn at when fitted, which CSS decides and this has to
 * agree with. Never enlarged: `max-width: 100%` only ever shrinks.
 */
export function fittedIn(picture: Size, frame: Size): Size {
  if (picture.width <= 0 || picture.height <= 0) return { width: 0, height: 0 };
  const scale = Math.min(frame.width / picture.width, frame.height / picture.height, 1);
  return { width: picture.width * scale, height: picture.height * scale };
}

/**
 * The same view, with the picture kept inside its frame. The slack is what the
 * picture has that the frame does not, halved because the transform measures
 * from the centre. Without this the picture can be flung off the screen with no
 * gesture that says "come back".
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
 * Zoom by a factor about a point, leaving what is under that point where it is
 * — the whole feel of a pinch; scaling about the centre slides the thing you
 * were looking at away. The factor is recomputed after clamping, so a pinch
 * running into [[CLOSEST]] stops magnifying without also sliding.
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
 * One tap: in about the point, or back to the whole picture. Out is [[FIT]]
 * exactly, not the reverse of the way in: undoing by dividing leaves the offset
 * it was dragged to.
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
  // distance jumps to CLOSEST on the first frame.
  return { at: middle(now), by: before > 0 ? apart(now) / before : 1 };
}
