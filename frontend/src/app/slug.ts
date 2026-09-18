import { Pipe, type PipeTransform } from '@angular/core';

/**
 * A memory's slug, with a break opportunity after each `_`.
 *
 * ⚠ **CSS cannot do this.** `overflow-wrap: anywhere` is what stops an
 * unbreakable slug running off a 412px screen, and it breaks at the last
 * character that fits — so `feedback_no_magic_strings_use_upstream_taxonomy`
 * came out as `…taxonom` / `y`, a lone letter that reads as a rendering fault.
 * `_` is not a break opportunity to any engine, and no property adds one.
 *
 * ⚠ **`<wbr>`, not a zero-width space.** Both offer the break; only the element
 * leaves `textContent` alone. U+200B would ride along in every copied slug, and
 * a slug is copied to be pasted into a command.
 *
 * Applied where a MEMORY NAME is shown, and deliberately not to the reader's
 * paths and commands — those break at `/` already, and a shell command split at
 * an underscore would read as a different command. Nor to the graph's trail,
 * which is `nowrap` on purpose and scrolls.
 */
@Pipe({ name: 'slug' })
export class Slug implements PipeTransform {
  transform(name: string): string {
    return name.replaceAll('_', '_<wbr>');
  }
}
