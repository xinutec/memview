import { Pipe, PipeTransform } from '@angular/core';

/**
 * The time of day something happened, in the reader's own timezone. Hours and
 * minutes only: the day is the divider [[fold]] puts between entries. A pure
 * pipe, memoised on its input, so a change-detection pass does not re-format
 * the whole conversation.
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
