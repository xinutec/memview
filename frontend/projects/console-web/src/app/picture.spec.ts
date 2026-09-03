import { describe, expect, it } from 'vitest';

import { LONGEST, fetchable, fetchedAt, fitted, pointedAt, pictorial, weight } from './picture';

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

  it('recognises a render named by where it is on the disk', () => {
    // ⚠ **The shape observe actually wrote**, and the one that did nothing: a
    // session has the file it just rendered, and only has a URL for it if it
    // also happens to be running a server.
    expect(pictorial('/Users/example/Code/observe/data/peek/lroom-at20s-render.png')).toBe(true);
  });

  it('leaves alone what the console could not fetch anyway', () => {
    // ⚠ **This asserted `file:` was left alone, and the reason it gave was
    // true**: the console refused it too, so rewriting the link would have made
    // a tap fail where it used to work. Both ends have been corrected together
    // (memview#1373) — `images::fetch` now reads a `file:` URL as the path it
    // names, so the app may rewrite it and the console will serve it.
    expect(pictorial('not a url at all')).toBe(false);
    // ⚠ The console's own routes are not places on a disk. Rewriting one would
    // send the console to fetch itself.
    expect(pictorial('/api/sessions/s1/images/2026-08-05.png')).toBe(false);
  });

  it('says nothing about a page, which keeps its own behaviour', () => {
    expect(pictorial('https://example.invalid/tasks/1323')).toBe(false);
  });
});

describe('fetchable', () => {
  it('takes every shape a session writes, and only those', () => {
    // An address when it was serving, a path when it simply has the file, and
    // the path with a scheme on it — which is what `coach` writes.
    expect(fetchable('http://h:8917/data/peek/a.png')).toBe(true);
    expect(fetchable('/Users/example/render.dat')).toBe(true);
    expect(fetchable('file:///Volumes/example/render/out/soft_squat_left.png')).toBe(true);
    expect(fetchable('/api/state')).toBe(false);
    expect(fetchable('mailto:someone@example.invalid')).toBe(false);
  });

  /**
   * ⚠ **This asserted `file:///etc/passwd` was refused, and read as a guard it
   * never was.** The hostile example made the refusal look like protection —
   * but `/etc/passwd` written bare has always been fetchable, one test up, so
   * the scheme kept nothing out. What actually stops that file reaching anybody
   * is the sniff at the far end: it is not a PNG, JPEG, GIF or WebP, so the
   * console answers a sentence and no bytes.
   *
   * The old assertion cost `coach` three dead picture links (memview#1373). **A
   * test whose example is chosen to look dangerous can pin a rule that does no
   * work**, and its passing says nothing about the rule you think you have.
   */
  it('does not pretend the scheme is what guards a file nobody may read', () => {
    expect(fetchable('/etc/passwd')).toBe(true);
    expect(fetchable('file:///etc/passwd')).toBe(true);
    // Neither is offered as a picture, which is the check that was doing the
    // work the scheme was being credited with.
    expect(pictorial('/etc/passwd')).toBe(false);
    expect(pictorial('file:///etc/passwd')).toBe(false);
  });

  it('reads a file: link as the picture it is', () => {
    // Verbatim shape from `coach`, 2026-09-03: three of these, every one dead
    // while the identical path without the scheme served 200 image/png.
    expect(pictorial('file:///Volumes/example/render/out/soft_squat_left.png')).toBe(true);
    // The path decides, and it is percent-decoded by `new URL` before the
    // ending is read — a render whose name has a space in it is still a render.
    expect(pictorial('file:///Volumes/example/out/soft%20squat.png')).toBe(true);
  });

  it('is what an explicit image asks, so a render need not be named for what it is', () => {
    // `![alt](…)` is the author saying it is a picture; the extension has
    // nothing left to decide. `pictorial` stays stricter for a plain link.
    expect(fetchable('/Users/example/data/peek/at20s')).toBe(true);
    expect(pictorial('/Users/example/data/peek/at20s')).toBe(false);
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
