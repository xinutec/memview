import { TestBed } from '@angular/core/testing';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { BACKOFF_MS, Restyle } from './restyle';

/** The stylesheet links currently in the head, newest last. */
function sheets(): HTMLLinkElement[] {
  return [...document.head.querySelectorAll<HTMLLinkElement>('link[rel="stylesheet"]')];
}

/** A stylesheet in the head that is about to fail, as the parser would leave it. */
function failing(href: string): HTMLLinkElement {
  const link = document.createElement('link');
  link.rel = 'stylesheet';
  link.href = href;
  document.head.appendChild(link);
  return link;
}

describe('Restyle', () => {
  let restyle: Restyle;

  beforeEach(() => {
    vi.useFakeTimers();
    for (const link of sheets()) link.remove();
    window.brokenAssets = [];
    // ⚠ Without this the root injector hands back the SAME service to every
    // test, so each `init` leaves another live listener on the window and one
    // failure is answered several times over. That is a real hazard rather than
    // a test artefact — hence the guard in `init` — but the tests still have to
    // start from one service each or they measure the leak instead.
    TestBed.resetTestingModule();
    restyle = TestBed.inject(Restyle);
  });

  afterEach(() => {
    vi.useRealTimers();
    for (const link of sheets()) link.remove();
  });

  it('asks again for a stylesheet that broke before the app booted', () => {
    // ⚠ The case that actually happened. Stylesheets are requested while the
    // HTML is parsed and module scripts run after it, so the app boots into a
    // page that is already unstyled and no error is left to hear. The inline
    // recorder in index.html is the only witness; this is what acts on it.
    window.brokenAssets = ['styles-YGANRVWE.css'];

    restyle.init();
    vi.advanceTimersByTime(BACKOFF_MS[0]);

    expect(sheets().map((link) => link.getAttribute('href'))).toEqual([
      'styles-YGANRVWE.css?again=1',
    ]);
  });

  it('waits before asking, rather than retrying into the same dropped connection', () => {
    window.brokenAssets = ['styles.css'];

    restyle.init();
    expect(sheets()).toEqual([]);

    vi.advanceTimersByTime(BACKOFF_MS[0]);
    expect(sheets()).toHaveLength(1);
  });

  it('asks again for one that breaks after boot', () => {
    restyle.init();
    const link = failing('styles.css');

    link.dispatchEvent(new Event('error'));
    vi.advanceTimersByTime(BACKOFF_MS[0]);

    expect(sheets().map((link) => link.getAttribute('href'))).toContain('styles.css?again=1');
  });

  it('gives up rather than asking forever', () => {
    // A file that is genuinely gone will not arrive, and a page re-requesting it
    // for the rest of the session is worse than one that has stopped: it is a
    // request per attempt against a runner that already said no.
    restyle.init();
    let failed = failing('styles.css');
    const asked: string[] = [];
    for (let round = 1; round <= BACKOFF_MS.length + 2; round++) {
      failed.dispatchEvent(new Event('error'));
      vi.advanceTimersByTime(BACKOFF_MS.at(-1)!);
      const fresh = sheets().find((link) => link.dataset['again'] === String(round));
      if (fresh) {
        asked.push(fresh.getAttribute('href') ?? '');
        failed = fresh;
      }
    }

    expect(asked).toEqual(['styles.css?again=1', 'styles.css?again=2', 'styles.css?again=3']);
    // The two rounds past the bound left nothing behind: only the original,
    // which is inert but harmless, and the last replacement to have failed.
    expect(sheets().map((link) => link.getAttribute('href'))).toEqual(['styles.css']);
  });

  it('counts attempts by the stylesheet, not by the address it was asked for', () => {
    // Each retry carries a different query, so counting whole addresses would
    // make every attempt look like a new file and the bound would never be met.
    restyle.init();
    const link = failing('styles.css');
    link.dispatchEvent(new Event('error'));
    vi.advanceTimersByTime(BACKOFF_MS[0]);

    const second = sheets().at(-1)!;
    second.dispatchEvent(new Event('error'));
    vi.advanceTimersByTime(BACKOFF_MS[1]);

    expect(sheets().map((link) => link.getAttribute('href'))).toEqual([
      'styles.css',
      'styles.css?again=2',
    ]);
  });
});
