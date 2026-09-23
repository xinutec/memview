import { diffLines } from 'diff';

import type { Change } from './models';

/** One line of a change as a diff draws it. */
export interface Line {
  readonly kind: 'same' | 'gone' | 'added';
  readonly text: string;
}

/** The lines of a change: unchanged, removed and added, removed first where they differ. */
export function lines(change: Change): Line[] {
  return diffLines(change.before, change.after).flatMap((part) => {
    const kind = part.added ? 'added' : part.removed ? 'gone' : 'same';
    return part.value
      .replace(/\n$/, '')
      .split('\n')
      .map((text) => ({ kind, text }));
  });
}
