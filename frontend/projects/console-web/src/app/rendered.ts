import { Pipe, PipeTransform, SecurityContext, inject } from '@angular/core';
import { DomSanitizer } from '@angular/platform-browser';
import { marked } from 'marked';

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
    const html = marked.parse(text, { async: false, gfm: true, breaks: true });
    return this.sanitizer.sanitize(SecurityContext.HTML, html) ?? '';
  }
}
