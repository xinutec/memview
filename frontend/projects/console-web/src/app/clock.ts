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

/**
 * A time written into a sentence: the hour and minute in the reader's own
 * locale — and the second, for times seconds apart — with the day in front when
 * it was not today.
 */
export function clock(at: number, now: Date = new Date(), seconds = false): string {
  const then = new Date(at);
  const time = then.toLocaleTimeString(undefined, {
    hour: '2-digit',
    minute: '2-digit',
    ...(seconds && { second: '2-digit' }),
  });
  return then.toDateString() === now.toDateString()
    ? time
    : `${then.toLocaleDateString(undefined, { day: 'numeric', month: 'short' })} ${time}`;
}
