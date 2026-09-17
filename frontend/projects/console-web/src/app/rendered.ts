import { Pipe, PipeTransform, SecurityContext, inject } from '@angular/core';
import { DomSanitizer } from '@angular/platform-browser';
import { Marked, type Tokens } from 'marked';

import { fetchable, fetchedAt, pictorial } from './picture';

/**
 * A checked and an unchecked task, as characters: the sanitiser strips the
 * `<input>` GFM emits, which left the two states indistinguishable. Characters
 * survive being copied out of the page, which a CSS box does not.
 */
const TICKED = '☑';
const UNTICKED = '☐';

/**
 * The class a picture link carries, and what session-view watches for. In the
 * class rather than a `data-` attribute, which the sanitiser and `[innerHTML]`
 * would strip. Not `picture`, which is the button around a sent picture.
 */
export const PICTURE = 'picture-link';

/** `<`, `&` and the quotes, for text going into an attribute. */
function attribute(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

/**
 * marked, with the two things it renders that this app renders differently. An
 * instance, so a renderer override cannot leak into any other use.
 */
const renderer = new Marked({
  gfm: true,
  breaks: true,
  renderer: {
    listitem(item: Tokens.ListItem): string {
      const body = this.parser.parse(item.tokens);
      if (!item.task) return `<li>${body}</li>`;
      // No space added: GFM's tokeniser leaves the one that followed the `]`.
      return `<li class="task">${item.checked ? TICKED : UNTICKED}${body}</li>`;
    },

    /**
     * A link to a picture points at the console; every other link is untouched.
     * GFM already made these anchors, and tapping one handed the address to a
     * browser that cannot reach the LAN it names — see [[pictorial]]. The `href` is
     * rewritten so the tap has somewhere to go if the handler misses it; the text
     * is left as written.
     */
    link(token: Tokens.Link): string {
      const body = this.parser.parseInline(token.tokens);
      const titled = token.title ? ` title="${attribute(token.title)}"` : '';
      if (!pictorial(token.href)) return `<a href="${attribute(token.href)}"${titled}>${body}</a>`;
      return `<a class="${PICTURE}" href="${attribute(fetchedAt(token.href))}"${titled}>${body}</a>`;
    },

    /**
     * `![alt](url)` is drawn as the same link, not an `<img>`: a transcript that
     * inlines pictures fetches a megabyte per render as it scrolls, down a tunnel to
     * a phone. [[fetchable]] here, where a plain link asks [[pictorial]]: the `!` is
     * the author saying this is a picture.
     */
    image(token: Tokens.Image): string {
      const label = attribute(token.text || token.href);
      if (!fetchable(token.href)) return `<a href="${attribute(token.href)}">${label}</a>`;
      return `<a class="${PICTURE}" href="${attribute(fetchedAt(token.href))}">${label}</a>`;
    },
  },
});

/**
 * A message as its author wrote it — tables, headings, code and all. Rendered
 * on the client because text streams as deltas and is assembled here.
 * Sanitised even though the source is trusted: a model's output is whatever a
 * tool result or a web page put in front of it. `marked` stopped sanitising in
 * v5, so `SecurityContext.HTML` does it; `bypassSecurityTrustHtml` is exactly
 * what this must not do.
 */
@Pipe({ name: 'rendered' })
export class Rendered implements PipeTransform {
  private sanitizer = inject(DomSanitizer);

  transform(text: string | undefined): string {
    if (!text) return '';
    // Synchronous: `async: false` makes it a type-level fact.
    const html = renderer.parse(text, { async: false });
    return this.sanitizer.sanitize(SecurityContext.HTML, html) ?? '';
  }
}
