import { describe, expect, it } from 'vitest';

import { colour } from './coloured';

const ESC = '\x1b';

describe('ANSI in tool output', () => {
  it('turns a colour code into a class rather than showing the code', () => {
    // ⚠ **The shape Pippijn actually sees.** vitest writes its summary in SGR,
    // and with nothing reading it the ESC byte is invisible while the bracket
    // codes are not — so the phone showed `[2m Test Files [22m [1m[32m22 passed`.
    expect(colour(`${ESC}[32m22 passed${ESC}[39m`)).toBe('<span class="ansi-fg-2">22 passed</span>');
    expect(colour(`${ESC}[32m22 passed${ESC}[39m`)).not.toContain('[32m');
  });

  it('carries bold and dim, which is most of what a build log uses', () => {
    expect(colour(`${ESC}[1mloud${ESC}[22m`)).toBe('<span class="ansi-b">loud</span>');
    expect(colour(`${ESC}[2mquiet${ESC}[22m`)).toBe('<span class="ansi-d">quiet</span>');
  });

  it('combines a weight and a colour in one span', () => {
    expect(colour(`${ESC}[1;31mbad${ESC}[0m`)).toBe('<span class="ansi-b ansi-fg-1">bad</span>');
  });

  it('closes an unterminated run at the end of the output', () => {
    // A log cut mid-line is the normal case — `cut` says so on the entry — and
    // an unclosed span would colour the rest of the page.
    expect(colour(`${ESC}[31mtruncated`)).toBe('<span class="ansi-fg-1">truncated</span>');
  });

  it('escapes the text it is given, before any of its own markup', () => {
    // ⚠ **Tool output is a file, a web page, or a model's words.** It is not a
    // threat model, but it is not ours either, and this is the one thing that
    // turns it into HTML.
    expect(colour('<script>alert(1)</script>')).toBe('&lt;script&gt;alert(1)&lt;/script&gt;');
    expect(colour(`${ESC}[31m<b>x</b>`)).toBe('<span class="ansi-fg-1">&lt;b&gt;x&lt;/b&gt;</span>');
  });

  it('drops an escape it does not draw rather than printing it', () => {
    // Cursor moves and erases are a terminal talking to itself. Drawn, they are
    // line noise; left as codes, they are worse.
    expect(colour(`${ESC}[2Kclean${ESC}[0K`)).toBe('clean');
    expect(colour(`${ESC}]0;a title\x07still here`)).toBe('still here');
  });

  it('lets a carriage return overwrite the line, as a terminal would', () => {
    // ⚠ **Progress bars, which build logs are made of.** `cargo` and `pnpm`
    // rewrite one line hundreds of times; printed in full they bury the output
    // that matters under their own history.
    expect(colour('10%\r50%\r100% done')).toBe('100% done');
    expect(colour('first\nold\rnew')).toBe('first\nnew');
  });

  it('leaves plain text exactly as it was', () => {
    expect(colour('nothing to see')).toBe('nothing to see');
    expect(colour('')).toBe('');
  });

  it('reads a bright colour as its own shade, not as bold', () => {
    // 90-97 are the bright set. Folding them onto 30-37 plus bold is a common
    // shortcut and loses the difference between grey and black.
    expect(colour(`${ESC}[90mgrey${ESC}[39m`)).toBe('<span class="ansi-fg-8">grey</span>');
  });

  it('takes a 256-colour and a truecolour code without drawing them wrong', () => {
    // Not supported, and skipped WHOLE: reading `38;5;196` as `38` then `5`
    // then `196` would set some other colour entirely and look deliberate.
    expect(colour(`${ESC}[38;5;196mred-ish${ESC}[0m`)).toBe('red-ish');
    expect(colour(`${ESC}[38;2;255;0;0mred-ish${ESC}[0m`)).toBe('red-ish');
  });

  it('carries a background colour separately from a foreground one', () => {
    expect(colour(`${ESC}[41mred behind${ESC}[49m`)).toBe('<span class="ansi-bg-1">red behind</span>');
  });
});
