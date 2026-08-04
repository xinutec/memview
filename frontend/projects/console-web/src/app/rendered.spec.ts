import { beforeEach, describe, expect, it } from 'vitest';
import { TestBed } from '@angular/core/testing';

import { Rendered } from './rendered';

/**
 * What the markdown pipeline actually produces, measured rather than assumed.
 *
 * ⚠ **Two layers, and the sanitiser is the one that surprises.** `marked` is
 * given GFM, so it emits everything the syntax promises — and then Angular's
 * sanitiser removes whatever it will not allow into the page, silently. The
 * pair is what a reader sees, so the pair is what these test.
 *
 * The alignment half cannot be tested here: an attribute the sanitiser keeps is
 * still only honoured if the app's CSS lets it through, and jsdom has no
 * cascade. That one is measured in the layout harness against a rendered page.
 */
describe('Rendered', () => {
  let render: (text: string) => string;

  beforeEach(() => {
    TestBed.configureTestingModule({ providers: [Rendered] });
    const pipe = TestBed.inject(Rendered);
    render = (text: string) => pipe.transform(text);
  });

  /** What a reader sees, not what the string says. ⚠ The sanitiser returns the
   *  marks as numeric entities — `&#9745;` — so asserting on the HTML would be
   *  asserting on its encoding rather than on whether anything is legible. */
  const shown = (html: string): string => {
    const holder = document.createElement('div');
    holder.innerHTML = html;
    return holder.textContent ?? '';
  };

  it('keeps a task list distinguishable once the checkbox is gone', () => {
    // The defect this replaces: GFM emits `<input type="checkbox">`, the
    // sanitiser strips it, and `- [x] done` and `- [ ] not` both rendered as a
    // bullet with a leading space. Two different states, one appearance.
    const html = render('- [x] done\n- [ ] not\n');
    expect(shown(html)).toContain('☑ done');
    expect(shown(html)).toContain('☐ not');
    expect(html).not.toContain('<input');
    // And they are still a list, not a paragraph of symbols.
    expect(html).toContain('<li');
  });

  it('marks a task in a loose list too, where the body is a block', () => {
    // A blank line between items makes each body a `<p>`, so the mark lands
    // beside a block rather than beside a run of text. Recorded rather than
    // special-cased: it still says which state the item is in, which is the
    // whole job, and a list written that way is rare.
    const seen = shown(render('- [x] done\n\n- [ ] not\n'));
    expect(seen).toContain('☑');
    expect(seen).toContain('☐');
  });

  it('leaves an ordinary list item alone', () => {
    const html = render('- plain\n');
    expect(html).not.toContain('☐');
    expect(html).not.toContain('class="task"');
  });

  it('renders a table, with what is inside its cells', () => {
    const html = render('| what | why |\n|---|---|\n| `flex: 1` | **bold** |\n');
    expect(html).toContain('<table');
    expect(html).toContain('<code>flex: 1</code>');
    expect(html).toContain('<strong>bold</strong>');
  });

  it('carries the alignment a table asked for as far as the markup', () => {
    // Half the fix. Whether it survives the cascade is the harness's question.
    const html = render('| l | r |\n|:---|---:|\n| 1 | 2 |\n');
    expect(html).toContain('align="right"');
  });

  it('lets a table interrupt a paragraph, which is how they get written', () => {
    const html = render('Here it is:\n| a | b |\n|---|---|\n| 1 | 2 |\n');
    expect(html).toContain('<table');
  });

  it('strips a script and an event handler but keeps the words', () => {
    expect(render('x<script>alert(1)</script>y\n')).toContain('xy');
    const img = render('<img src=x onerror="alert(1)">\n');
    expect(img).not.toContain('onerror');
  });
});
