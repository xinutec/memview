/**
 * Indexing for the specs, under `noUncheckedIndexedAccess`.
 *
 * A spec that reads `entries[0]` is asserting the code under test produced one,
 * so the absence is a test failure with a message, not a `!` that turns into
 * `undefined is not an object` three lines later.
 *
 * The console has its own copy: the two apps share no code by design, and these
 * are three functions whose behaviour their names fully describe.
 */
export function nth<T>(items: readonly T[], index: number): T {
  const item = items[index];
  if (item === undefined) throw new Error(`no item at ${index}: the list holds ${items.length}`);
  return item;
}

export const first = <T>(items: readonly T[]): T => nth(items, 0);
export const last = <T>(items: readonly T[]): T => nth(items, items.length - 1);
