import { Pipe, PipeTransform, SecurityContext, inject } from '@angular/core';
import { DomSanitizer } from '@angular/platform-browser';

/**
 * Tool output as the terminal would have drawn it.
 *
 * The console shows what a command printed, and a great deal of what the fleet
 * runs prints in colour: `cargo`, `vitest`, `playwright`, `git`, `eslint`. With
 * nothing reading those codes the ESC byte is invisible and the rest is not, so
 * a test summary arrived on the phone as
 * `[2m Test Files [22m [1m[32m22 passed[39m[22m[90m (22)[39m` — the numbers
 * are all there and the line is unreadable, which on a four-inch screen is the
 * same as absent.
 *
 * ## Classes, not inline styles
 *
 * Angular's sanitiser strips a `style` attribute, so a colour written that way
 * would arrive as plain text with extra markup around it and no colour at all.
 * The classes here are defined in `session-view.scss` against the theme, which
 * is also what lets the same output stay legible in light and dark.
 *
 * ## Escaped first, and by us
 *
 * A tool result is a file, a web page, or a model's words — not a threat model,
 * but not ours either. The text is HTML-escaped before any span is added, and
 * the result still goes through `SecurityContext.HTML` on the way out.
 * `bypassSecurityTrustHtml` would also have compiled, and is exactly the thing
 * this must not do.
 */
@Pipe({ name: 'coloured' })
export class Coloured implements PipeTransform {
  private sanitizer = inject(DomSanitizer);

  transform(text: string | undefined): string {
    if (!text) return '';
    return this.sanitizer.sanitize(SecurityContext.HTML, colour(text)) ?? '';
  }
}

/**
 * The class prefixes, as literal strings on purpose.
 *
 * ⚠ **A dynamically-completed family is dead to a static checker unless it can
 * see the stem.** `DL-ANGULAR-DEAD-STYLE` reads what the templates and TS
 * reference and reports SCSS nothing uses; these classes are built here from a
 * number, so the rule found sixteen dead rules until the names carried a prefix
 * it could match. Working with that rather than waiving it is also the better
 * name: `ansi-fg-1` says what it is where `fg1` did not.
 */
const FG = 'ansi-fg-';
const BG = 'ansi-bg-';

/** Every escape sequence, so the ones we do not draw can be dropped whole. */
const SEQUENCE = /\x1b(?:\[[0-9;?]*[ -/]*[@-~]|\][^\x07\x1b]*(?:\x07|\x1b\\)?|[@-Z\\-_])/g;

/** A `\r` and everything it overwrote, within one line. */
const OVERWRITTEN = /^.*\r(?!\n)/gm;

/**
 * Text with its SGR codes turned into spans and its other escapes removed.
 *
 * Separate from the pipe so it can be tested without Angular's injector, and
 * because what it does — read a byte stream, keep only what it can account for —
 * is the same discipline as the rest of this repo.
 */
export function colour(text: string): string {
  // ⚠ **Before anything else, and on the whole string.** A `\r` that survives
  // into the output is drawn as nothing and leaves the overwritten text behind
  // it: a `cargo` build then reads as a hundred progress bars stacked up. The
  // negative lookahead keeps `\r\n`, which is a line ending and overwrites
  // nothing.
  const lines = text.replace(OVERWRITTEN, '');

  let html = '';
  let open = false;
  let at = 0;
  let active: string[] = [];

  const close = () => {
    if (open) html += '</span>';
    open = false;
  };
  const write = (piece: string) => {
    if (piece) html += escaped(piece);
  };

  for (const found of lines.matchAll(SEQUENCE)) {
    const index = found.index ?? 0;
    write(lines.slice(at, index));
    at = index + found[0].length;

    // Only SGR — `ESC [ … m` — says anything about how the text looks. Every
    // other sequence moves a cursor or talks to the terminal emulator, and this
    // is not one: dropping them is the whole of what they mean here.
    const sgr = /^\x1b\[([0-9;]*)m$/.exec(found[0]);
    if (!sgr) continue;

    const next = restyled(active, sgr[1]);
    if (next.join(' ') === active.join(' ')) continue;
    close();
    active = next;
    if (active.length) {
      html += `<span class="${active.join(' ')}">`;
      open = true;
    }
  }
  write(lines.slice(at));
  // ⚠ **A run left open would colour the rest of the page.** Output arrives
  // truncated as a matter of course — the entry carries `cut` to say so — and
  // the last thing in it is very often a code with no reset after it.
  close();
  return html;
}

/** What the classes become after one SGR sequence. */
function restyled(active: string[], parameters: string): string[] {
  // `ESC[m` is `ESC[0m`, and an empty parameter anywhere means zero.
  const codes = (parameters || '0').split(';').map((code) => Number(code || 0));
  let next = [...active];
  for (let i = 0; i < codes.length; i++) {
    const code = codes[i];
    // ⚠ **Skipped WHOLE, never read one number at a time.** `38;5;196` is one
    // instruction; taking `38` and then reading `5` and `196` as codes of their
    // own sets a different colour and an unrelated attribute, and looks
    // deliberate. Extended colour is not drawn here, so it is stepped over.
    if (code === 38 || code === 48) {
      i += codes[i + 1] === 5 ? 2 : codes[i + 1] === 2 ? 4 : 0;
      continue;
    }
    next = applied(next, code);
  }
  return next;
}

/**
 * One SGR code against the classes in force.
 *
 * ⚠ **The classes are numbered as the palette is** — 0 black through 7 white,
 * 8-15 the bright set — so the arithmetic is `code - 30`, not `code - 29`. An
 * off-by-one here does not fail: it silently draws every colour as its
 * neighbour, and red reads as green.
 */
function applied(active: string[], code: number): string[] {
  const without = (...classes: string[]) => active.filter((it) => !classes.includes(it));
  const weights = ['ansi-b', 'ansi-d'];
  switch (true) {
    case code === 0:
      return [];
    case code === 1:
      return [...without(...weights), 'ansi-b'];
    case code === 2:
      return [...without(...weights), 'ansi-d'];
    case code === 3:
      return [...without('ansi-i'), 'ansi-i'];
    case code === 4:
      return [...without('ansi-u'), 'ansi-u'];
    // 22 ends both weights at once, which is why they share a group above.
    case code === 22:
      return without(...weights);
    case code === 23:
      return without('ansi-i');
    case code === 24:
      return without('ansi-u');
    case code === 39:
      return active.filter((it) => !it.startsWith(FG));
    case code === 49:
      return active.filter((it) => !it.startsWith(BG));
    case code >= 30 && code <= 37:
      return [...active.filter((it) => !it.startsWith(FG)), `${FG}${code - 30}`];
    case code >= 40 && code <= 47:
      return [...active.filter((it) => !it.startsWith(BG)), `${BG}${code - 40}`];
    // The bright set is eight more shades, not the ordinary eight in bold: read
    // as bold, `90` — the grey every build log dims its noise with — comes out
    // as heavy black, which is the opposite emphasis.
    case code >= 90 && code <= 97:
      return [...active.filter((it) => !it.startsWith(FG)), `${FG}${code - 82}`];
    case code >= 100 && code <= 107:
      return [...active.filter((it) => !it.startsWith(BG)), `${BG}${code - 92}`];
    default:
      return active;
  }
}

/** The four characters that would otherwise be markup. */
function escaped(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}
