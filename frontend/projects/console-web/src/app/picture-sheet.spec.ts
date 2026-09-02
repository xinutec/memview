import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { MAT_BOTTOM_SHEET_DATA, MatBottomSheetRef } from '@angular/material/bottom-sheet';
import { TestBed } from '@angular/core/testing';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { fetchedAt, pointedAt } from './picture';
import { PictureSheet } from './picture-sheet';

const AT = 'http://10.0.0.2:8917/data/peek/peekA-350-view_top_down.png';

/**
 * jsdom has no blob URLs, so the two statics are supplied.
 *
 * ⚠ **The two statics, and not the global.** The first cut replaced `URL`
 * wholesale with `{...URL, createObjectURL, revokeObjectURL}` — a plain object,
 * so every `new URL(…)` in the app under test threw, and `pointedAt` quietly
 * answered `undefined` for a link that was perfectly well formed. The test that
 * caught it was testing something else.
 *
 * ⚠ **The stub still answers `blob:`**, which is the part being measured: whether
 * Angular's `[src]` sanitiser lets that scheme reach the element. A stub
 * returning `x` would pass while the real app showed nothing.
 */
const made: string[] = [];
const revoked: string[] = [];

beforeEach(() => {
  made.length = 0;
  revoked.length = 0;
  URL.createObjectURL = (blob: Blob) => {
    const at = `blob:console/${made.length}-${blob.size}`;
    made.push(at);
    return at;
  };
  URL.revokeObjectURL = (at: string) => void revoked.push(at);
});

afterEach(() => {
  // Taken away rather than put back: jsdom defines neither of them, measured, so
  // deleting is what restores the environment the next file gets.
  Reflect.deleteProperty(URL, 'createObjectURL');
  Reflect.deleteProperty(URL, 'revokeObjectURL');
});

async function open(): Promise<{ host: Element; http: HttpTestingController; done: () => void }> {
  await TestBed.configureTestingModule({
    imports: [PictureSheet],
    providers: [
      provideHttpClient(),
      provideHttpClientTesting(),
      { provide: MAT_BOTTOM_SHEET_DATA, useValue: { url: AT } },
      { provide: MatBottomSheetRef, useValue: { dismiss: () => undefined } },
    ],
  }).compileComponents();
  const fixture = TestBed.createComponent(PictureSheet);
  await fixture.whenStable();
  const host: unknown = fixture.nativeElement;
  if (!(host instanceof Element)) throw new Error('the fixture rendered no element');
  return {
    host,
    http: TestBed.inject(HttpTestingController),
    done: () => fixture.destroy(),
  };
}

describe('the picture sheet', () => {
  it('asks the console for the picture, rather than the address it was written at', async () => {
    // ⚠ **The whole reason this exists.** The phone cannot reach the LAN those
    // addresses name; the Mac can, and the phone is already talking to it.
    const { http, done } = await open();

    const asked = http.expectOne(fetchedAt(AT));
    // Round-tripped rather than compared to a literal: what the anchor carries
    // and what the console reads back out are the same two functions.
    expect(pointedAt(asked.request.url)).toBe(AT);
    asked.flush(new Blob([new Uint8Array([1, 2, 3])]));
    done();
  });

  it('draws what arrived, through a scheme the sanitiser allows', async () => {
    const { host, http, done } = await open();
    http.expectOne(fetchedAt(AT)).flush(new Blob([new Uint8Array([1, 2])]));
    TestBed.tick();

    const img = host.querySelector('img');
    // Angular sanitises a bound `src`, and a scheme it dislikes is replaced with
    // `unsafe:` rather than refused — which draws nothing and reports nothing.
    expect(img?.getAttribute('src')).toBe(made[0]);
    expect(img?.getAttribute('src')).toMatch(/^blob:/);
    done();
  });

  it('says what the console said, not what the status was', async () => {
    // ⚠ **The failure arrives as a `Blob`, because the request asked for one.**
    // Without reading it back the person sees "the runner answered 502", which
    // names neither of the two things it actually means.
    const { host, http, done } = await open();

    http
      .expectOne(fetchedAt(AT))
      .flush(new Blob(['it answered 404 Not Found']), { status: 502, statusText: 'Bad Gateway' });
    // The blob is read asynchronously; two turns of the microtask queue.
    await Promise.resolve();
    await new Promise((wake) => setTimeout(wake, 0));
    TestBed.tick();

    expect(host.textContent).toContain('it answered 404 Not Found');
    expect(host.textContent).not.toContain('502');
    done();
  });

  it('gives the blob URL back when it closes', async () => {
    // Megabytes each, held by the document until they are revoked — and a
    // conversation about a reconstruction is opened a dozen times.
    const { http, done } = await open();
    http.expectOne(fetchedAt(AT)).flush(new Blob([new Uint8Array([1])]));
    TestBed.tick();

    done();

    expect(revoked).toEqual([made[0]]);
  });
});
