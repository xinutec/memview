import { Pipe, PipeTransform, SecurityContext, inject } from '@angular/core';
import { DomSanitizer } from '@angular/platform-browser';
import { Marked, type Tokens } from 'marked';

/**
 * A checked and an unchecked task, as characters rather than as controls.
 *
 * ⚠ **The sanitiser will always strip an `<input>`**, which is what GFM emits
 * for `- [x]`. Measured: both `- [x] done` and `- [ ] not` rendered as a plain
 * bullet with a leading space, so the two states were indistinguishable — worse
 * than not supporting task lists at all, because a list that says nothing still
 * looks like it is saying something.
 *
 * Characters rather than a styled span: they survive being copied out of the
 * page, which a box drawn in CSS does not, and this text is quoted into commit
 * messages and notes.
 */
const TICKED = '☑';
const UNTICKED = '☐';

/**
 * marked, with the one thing it renders that cannot survive sanitising replaced.
 *
 * An instance rather than the global `marked`, so a renderer override here
 * cannot leak into any other use of the library in this app.
 */
const renderer = new Marked({
  gfm: true,
  breaks: true,
  renderer: {
    listitem(item: Tokens.ListItem): string {
      const body = this.parser.parse(item.tokens);
      if (!item.task) return `<li>${body}</li>`;
      // No space added: GFM's own tokeniser leaves the one that followed the
      // `]`, and adding a second is visible.
      return `<li class="task">${item.checked ? TICKED : UNTICKED}${body}</li>`;
    },
  },
});

/**
 * A message as its author wrote it — tables, headings, code and all.
 *
 * The console showed the raw characters, so an answer arrived as a wall with
 * `## The state, concretely` and `| stream | window |` in it. The text was right
 * and unreadable, which on a four-inch screen is the same as wrong.
 *
 * ## Why the client renders it
 *
 * memview renders markdown server-side with comrak, because its content is files
 * that arrive whole. A session's text does not: it streams as deltas and is
 * assembled here, so rendering upstream would mean either re-rendering per delta
 * or two different paths for live and recorded text. One renderer on this side
 * gives the same result for both.
 *
 * ## Why it is sanitised even though we trust the source
 *
 * The text is a model's output, which is not a threat model — but it is also
 * whatever a tool result, a file on disk or a web page put in front of that
 * model, and any of those can contain a `<script>`. `marked` deliberately stopped
 * sanitising in v5, so Angular's sanitiser does it: `SecurityContext.HTML`
 * strips scripts and event handlers rather than trusting the string.
 *
 * `bypassSecurityTrustHtml` would also have compiled, and is exactly the thing
 * this must not do.
 */
@Pipe({ name: 'rendered' })
export class Rendered implements PipeTransform {
  private sanitizer = inject(DomSanitizer);

  transform(text: string | undefined): string {
    if (!text) return '';
    // Synchronous: `marked` can return a promise when extensions ask for it, and
    // this uses none. `async: false` makes that a type-level fact rather than a
    // hope.
    const html = renderer.parse(text, { async: false });
    return this.sanitizer.sanitize(SecurityContext.HTML, html) ?? '';
  }
}
