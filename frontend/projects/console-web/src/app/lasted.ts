import { Pipe, PipeTransform } from '@angular/core';

/**
 * How long something has been going, for a number that is still moving. Not
 * [[fold]]'s `elapsed`: this repaints once a second, so a tenth would flicker.
 * Zero-padded seconds past a minute, or the text shifts as it counts. A pure
 * pipe, memoised on its input.
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
