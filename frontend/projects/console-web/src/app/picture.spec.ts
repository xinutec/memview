import { describe, expect, it } from 'vitest';

import { LONGEST, fetchedAt, fitted, pointedAt, pictorial, weight } from './picture';

describe('fitted', () => {
  it('brings a phone screenshot down to the edge worth sending', () => {
    // A Pixel 9 screenshot, which is the commonest thing this will ever carry.
    expect(fitted(1080, 2400)).toEqual({ width: 706, height: LONGEST });
  });

  it('brings a photograph down the other way round', () => {
    expect(fitted(4080, 3072)).toEqual({ width: LONGEST, height: 1181 });
  });

  it('leaves a small picture alone rather than blowing it up', () => {
    // ⚠ Enlarging costs four times the bytes for exactly the same picture — the
    // detail to read is not there to be recovered.
    expect(fitted(400, 300)).toEqual({ width: 400, height: 300 });
  });

  it('keeps the shape it was given', () => {
    const square = fitted(3000, 3000);
    expect(square.width).toBe(square.height);
  });
});

describe('weight', () => {
  it('says kB up to a megabyte and MB after it', () => {
    expect(weight(180 * 1024)).toBe('180 kB');
    expect(weight(2.5 * 1024 * 1024)).toBe('2.5 MB');
  });

  it('says bytes below a kilobyte rather than nothing', () => {
    // `0 kB` beside a thumbnail reads as a picture that failed to load.
    expect(weight(700)).toBe('700 B');
  });
});

describe('pictorial', () => {
  it('recognises the renders a session writes links to', () => {
    // The shape actually in the observe transcript.
    expect(pictorial('http://10.0.0.2:8917/data/peek/peekA-350-view_top_down.png')).toBe(true);
    expect(pictorial('https://somewhere/a.JPEG')).toBe(true);
  });

  it('reads the path and not the whole address', () => {
    // ⚠ A search for a picture is not a picture. The extension has to be where
    // the file name is, or every query mentioning one opens an empty viewer.
    expect(pictorial('https://example.invalid/search?q=cat.png')).toBe(false);
    expect(pictorial('http://h/a.png?again=2')).toBe(true);
  });

  it('leaves alone what the console could not fetch anyway', () => {
    // `file:` is refused at the other end too — see `console::images::fetch` —
    // and a link the app rewrote but the console will not serve is a tap that
    // fails where it used to work.
    expect(pictorial('file:///home/example/render.png')).toBe(false);
    expect(pictorial('/local/page.png')).toBe(false);
    expect(pictorial('not a url at all')).toBe(false);
  });

  it('says nothing about a page, which keeps its own behaviour', () => {
    expect(pictorial('https://example.invalid/tasks/1323')).toBe(false);
  });
});

describe('fetchedAt and pointedAt', () => {
  it('carry an address through the query string and back', () => {
    // The pair is how a tap finds what to open: the address goes into the
    // anchor and comes back out of it, with nothing else to carry it.
    const at = 'http://h:8917/data/peek/a.png?again=2&view=top#part';
    expect(pointedAt(fetchedAt(at))).toBe(at);
  });

  it('encodes what would otherwise end the parameter', () => {
    // ⚠ `&` and `#` are the two that truncate silently: without encoding, the
    // console receives half an address and answers 404 about a file that exists.
    expect(fetchedAt('http://h/a.png?x=1&y=2')).toBe(
      '/api/picture?url=http%3A%2F%2Fh%2Fa.png%3Fx%3D1%26y%3D2',
    );
  });

  it('does not read an address out of a link that is not one of ours', () => {
    expect(pointedAt('/api/sessions/s1/images/2026-08-05.png')).toBeUndefined();
    expect(pointedAt('https://example.invalid/?url=http://h/a.png')).toBeUndefined();
  });
});
