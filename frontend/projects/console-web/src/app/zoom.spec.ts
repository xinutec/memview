import { describe, expect, it } from 'vitest';

import {
  CLOSEST,
  CLOSER,
  FIT,
  bounded,
  fittedIn,
  moved,
  pinched,
  scaledAbout,
  toggled,
  type Point,
  type Size,
} from './zoom';

/** A phone, and a render shaped like the ones observe writes. */
const FRAME: Size = { width: 412, height: 700 };
const PICTURE: Size = { width: 600, height: 800 };
/** What that render is drawn at when fitted: 412 wide, in proportion. */
const BASE = fittedIn(PICTURE, FRAME);

/** Where a point on the base picture ends up on screen, under a view. */
const shown = (view: { scale: number; x: number; y: number }, at: Point): Point => ({
  x: view.x + at.x * view.scale,
  y: view.y + at.y * view.scale,
});

describe('fittedIn', () => {
  it('fills the narrow axis and keeps the shape', () => {
    expect(BASE).toEqual({ width: 412, height: 549.3333333333334 });
  });

  it('leaves a small picture at its own size rather than blowing it up', () => {
    // ⚠ `max-width: 100%` only shrinks. A model that assumed the picture filled
    // the frame would let a thumbnail be dragged around a gap that is not there.
    expect(fittedIn({ width: 200, height: 100 }, FRAME)).toEqual({ width: 200, height: 100 });
  });

  it('says nothing rather than dividing by a picture that has not loaded', () => {
    expect(fittedIn({ width: 0, height: 0 }, FRAME)).toEqual({ width: 0, height: 0 });
  });
});

describe('bounded', () => {
  it('centres an axis with nothing to spare, however hard it is dragged', () => {
    // Fitted, the picture is exactly as wide as the frame and shorter than it:
    // there is no slack in either axis, so both stay at zero.
    expect(bounded({ scale: 1, x: 300, y: -400 }, FRAME, BASE)).toEqual(FIT);
  });

  it('lets it move exactly as far as it overhangs', () => {
    // At 2×, the picture is 824 wide in a 412 frame: 412 of overhang, half of it
    // reachable in each direction.
    const at = bounded({ scale: 2, x: 999, y: 0 }, FRAME, BASE);
    expect(at.x).toBe(206);
    expect(bounded({ scale: 2, x: -999, y: 0 }, FRAME, BASE).x).toBe(-206);
  });

  it('refuses to zoom out past the whole picture, or in past the limit', () => {
    expect(bounded({ scale: 0.2, x: 0, y: 0 }, FRAME, BASE).scale).toBe(1);
    expect(bounded({ scale: 99, x: 0, y: 0 }, FRAME, BASE).scale).toBe(CLOSEST);
  });
});

describe('scaledAbout', () => {
  it('leaves what is under the fingers where it is', () => {
    // ⚠ **The whole feel of a pinch.** Scaling about the centre instead lets the
    // thing being zoomed towards slide away from under the fingers.
    const at: Point = { x: 100, y: -150 };
    const view = scaledAbout(FIT, at, 2, FRAME, BASE);

    // The base point that was under `at` before is still under it after.
    const was = { x: (at.x - FIT.x) / FIT.scale, y: (at.y - FIT.y) / FIT.scale };
    expect(shown(view, was).x).toBeCloseTo(at.x, 6);
    expect(shown(view, was).y).toBeCloseTo(at.y, 6);
  });

  it('holds the anchor when the pinch runs into the limit', () => {
    // The factor is recomputed after clamping: without that, a pinch that has
    // stopped magnifying carries on sliding, which reads as the picture
    // escaping.
    const at: Point = { x: 80, y: 40 };
    const hard = scaledAbout({ scale: CLOSEST, x: 0, y: 0 }, at, 4, FRAME, BASE);
    expect(hard.scale).toBe(CLOSEST);
    expect(hard.x).toBe(0);
    expect(hard.y).toBe(0);
  });

  it('cannot be zoomed out past the whole picture', () => {
    expect(scaledAbout(FIT, { x: 50, y: 50 }, 0.1, FRAME, BASE)).toEqual(FIT);
  });
});

describe('moved', () => {
  it('pans when there is room and does nothing when there is not', () => {
    const close = scaledAbout(FIT, { x: 0, y: 0 }, 2, FRAME, BASE);
    expect(moved(close, { x: -50, y: 0 }, FRAME, BASE).x).toBe(-50);
    // Fitted, a drag has nowhere to go — and must not leave the picture parked
    // off-centre with no gesture that brings it back.
    expect(moved(FIT, { x: -50, y: -50 }, FRAME, BASE)).toEqual(FIT);
  });
});

describe('toggled', () => {
  it('goes in about the tap, and comes back to the whole picture exactly', () => {
    const at: Point = { x: 120, y: -60 };
    const close = toggled(FIT, at, FRAME, BASE);
    expect(close.scale).toBe(CLOSER);

    // ⚠ Out is FIT, not the way in reversed: dividing back would leave whatever
    // it had been dragged to, so the picture returns off-centre.
    const dragged = moved(close, { x: -40, y: 30 }, FRAME, BASE);
    expect(toggled(dragged, at, FRAME, BASE)).toEqual(FIT);
  });
});

describe('pinched', () => {
  it('reads the spread as the factor and the middle as the anchor', () => {
    const was = [
      { x: -50, y: 0 },
      { x: 50, y: 0 },
    ] as const;
    const now = [
      { x: -100, y: 0 },
      { x: 100, y: 20 },
    ] as const;

    const gesture = pinched(was, now);

    expect(gesture.by).toBeCloseTo(Math.hypot(200, 20) / 100, 6);
    expect(gesture.at).toEqual({ x: 0, y: 10 });
  });

  it('is not a pinch until the fingers are apart', () => {
    // ⚠ Two pointers reported at the same place divide by zero, and the viewer
    // jumps to its limit on the first frame of the gesture.
    const same = [
      { x: 10, y: 10 },
      { x: 10, y: 10 },
    ] as const;
    expect(
      pinched(same, [
        { x: 0, y: 0 },
        { x: 40, y: 0 },
      ] as const).by,
    ).toBe(1);
  });
});
