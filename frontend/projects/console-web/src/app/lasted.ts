import { Pipe, PipeTransform } from '@angular/core';

/**
 * How long something has been going, for a number that is still moving.
 *
 * ⚠ **Not [[fold]]'s `elapsed`, and the difference is the tenth of a second.**
 * That one measures a turn that has finished, where `48.2s` is a fact worth
 * stating precisely. This one is read while the thing is still happening and
 * repaints once a second, so a tenth would be a digit that is wrong for most of
 * its life and flickers for the rest.
 *
 * Zero-padded seconds past a minute — `2m 03s`, not `2m 3s` — because the number
 * is watched rather than read, and an unpadded one shifts the text sideways as
 * it counts.
 *
 * A pure pipe: it is read for every running row on every change-detection pass,
 * and memoising on the input is what keeps a ticking page from re-formatting the
 * whole transcript. See [[Clock]] for the same reasoning about times of day.
 */
@Pipe({ name: 'lasted' })
export class Lasted implements PipeTransform {
  transform(ms: number | undefined): string {
    if (ms === undefined || ms < 0) return '';
    const total = Math.floor(ms / 1000);
    if (total < 60) return `${total}s`;
    const minutes = Math.floor(total / 60);
    if (minutes < 60) return `${minutes}m ${String(total % 60).padStart(2, '0')}s`;
    return `${Math.floor(minutes / 60)}h ${String(minutes % 60).padStart(2, '0')}m`;
  }
}
