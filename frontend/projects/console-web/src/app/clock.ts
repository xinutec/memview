import { Pipe, PipeTransform } from '@angular/core';

/**
 * The time of day something happened, in the reader's own timezone.
 *
 * Hours and minutes only. Which *day* is answered by the date [[fold]] puts
 * between entries when the conversation crosses midnight, so repeating it on
 * every line would be noise on a screen that has none to spare.
 *
 * A pipe rather than a method, deliberately: the template reads this for every
 * entry on every change-detection pass, and a method would re-format the whole
 * conversation each time. A pure pipe is memoised on its input, and this one's
 * answer never changes for a given input — it asks nothing about now.
 */
@Pipe({ name: 'clock' })
export class Clock implements PipeTransform {
  transform(at: number | undefined): string {
    if (at === undefined) return '';
    return new Date(at).toLocaleTimeString(undefined, {
      hour: '2-digit',
      minute: '2-digit',
    });
  }
}
