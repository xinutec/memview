import { Pipe, PipeTransform, SecurityContext, inject } from '@angular/core';
import { DomSanitizer } from '@angular/platform-browser';
import { Marked, type Tokens } from 'marked';

import { fetchable, fetchedAt, pictorial } from './picture';

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
 * The class a picture link carries, and what session-view watches for.
 *
 * The mark travels in the class rather than in a `data-` attribute because this
 * string is sanitised on the way out and again by the `[innerHTML]` binding, and
 * a class is something both keep. The rest of what the tap needs is in the
 * `href`.
 *
 * ⚠ **Not `picture`, which is taken.** That is the button around a picture
 * somebody sent from the phone, and `session-view.scss` styles it and the `img`
 * inside it. Two unrelated things under one class name is a rule that reaches
 * something it was never written for, and an e2e selector that quietly matches
 * twice.
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
 * marked, with the two things it renders that this app renders differently.
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

    /**
     * A link to a picture points at the console; every other link is untouched.
     *
     * ⚠ **GFM already made these anchors** — a bare URL in a sentence is
     * autolinked, which is why tapping one on the phone did something rather
     * than nothing. What it did was hand the address to the browser, which
     * cannot reach the LAN it names. See [[pictorial]].
     *
     * The `href` is rewritten rather than only marked, so the tap has somewhere
     * to go if the handler ever misses it: the console's own origin is a host
     * the shell keeps in the app, where the original address is one it hands
     * away. The text of the link is left as it was written — the address is what
     * a person recognises the render by.
     */
    link(token: Tokens.Link): string {
      const body = this.parser.parseInline(token.tokens);
      const titled = token.title ? ` title="${attribute(token.title)}"` : '';
      if (!pictorial(token.href)) return `<a href="${attribute(token.href)}"${titled}>${body}</a>`;
      return `<a class="${PICTURE}" href="${attribute(fetchedAt(token.href))}"${titled}>${body}</a>`;
    },

    /**
     * `![alt](url)` is drawn as the same link, and not as an `<img>`.
     *
     * ⚠ **A transcript that inlines pictures is a transcript that fetches as it
     * scrolls.** Each one is a megabyte down a tunnel to a phone that may be on
     * cellular, arriving because somebody scrolled past a sentence — and the
     * pictures a session writes are renders it is iterating on, so a
     * conversation accumulates dozens. The links open in one tap and cost
     * nothing until they are asked for.
     *
     * The alt text is the label when there is one, because that is what it was
     * written to be.
     *
     * ⚠ **[[fetchable]] here, where a plain link asks [[pictorial]].** The `!`
     * is the author saying this is a picture, so an extension has nothing left
     * to decide — and what a session renders is not always named for what it
     * is. A link written without the `!` gets the stricter test, because most
     * links are not pictures and guessing wrong takes a page away from the
     * browser that could have shown it.
     */
    image(token: Tokens.Image): string {
      const label = attribute(token.text || token.href);
      if (!fetchable(token.href)) return `<a href="${attribute(token.href)}">${label}</a>`;
      return `<a class="${PICTURE}" href="${attribute(fetchedAt(token.href))}">${label}</a>`;
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
