import { TestBed } from '@angular/core/testing';
import { firstValueFrom } from 'rxjs';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as Y from 'yjs';

import { Drafts, difference } from './drafts';
import { Local } from './local';
import type { Picture } from './picture';

/**
 * Unsent words on this device, and what crosses to the runner.
 *
 * What the RUNNER does with a push is tested in Rust, in
 * `console/tests/suite/drafts.rs`, and the two meeting for real is
 * `e2e/two-devices.spec.ts`. What is tested here is this side: that a keystroke
 * becomes the smallest edit it can, that what comes back merges rather than
 * replaces, and that the runner's own bytes are never echoed at it.
 */

/** A scaled picture as `shrink` hands one over, small enough to read in a test. */
const PICTURE: Picture = {
  data: 'aGVsbG8=',
  mediaType: 'image/png',
  width: 100,
  height: 200,
  bytes: 5,
  preview: 'blob:http://localhost/8f0e',
};

/** One push as it reached the runner. */
interface Sent {
  readonly ulid: string;
  readonly update: string;
}

function harness(answer?: (url: string, init?: RequestInit) => Response): {
  drafts: Drafts;
  sent: Sent[];
} {
  const sent: Sent[] = [];
  const get = vi.fn((url: string | URL | Request, init?: RequestInit) => {
    const at = url instanceof Request ? url.url : String(url);
    if (init?.method === 'POST') {
      const body: unknown = JSON.parse(typeof init.body === 'string' ? init.body : '[]');
      if (Array.isArray(body)) {
        for (const one of body as Sent[]) sent.push(one);
      }
    }
    return Promise.resolve(
      answer?.(at, init) ??
        new Response(init?.method === 'POST' ? '[]' : '{"documents":[],"checkpoint":{"rev":0}}', {
          status: 200,
        }),
    );
  });
  const drafts = TestBed.inject(Drafts);
  drafts.configure(get);
  return { drafts, sent };
}

/** The text a pushed document holds. */
function reads(update: string): string {
  const binary = atob(update);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  const doc = new Y.Doc();
  Y.applyUpdate(doc, bytes);
  return doc.getText('text').toJSON();
}

/** A document somebody else wrote, base64, as the runner would hand it over. */
function elsewhere(text: string): string {
  const doc = new Y.Doc();
  doc.getText('text').insert(0, text);
  let binary = '';
  for (const byte of Y.encodeStateAsUpdate(doc)) binary += String.fromCharCode(byte);
  return btoa(binary);
}

/** What the runner answers a pull with, carrying one document. */
function holding(id: string, text: string): Response {
  return new Response(
    JSON.stringify({
      documents: [{ ulid: id, update: elsewhere(text), text, at: 1, rev: 1 }],
      checkpoint: { rev: 1 },
    }),
    { status: 200 },
  );
}

const words = (drafts: Drafts, id: string): Promise<string> => firstValueFrom(drafts.text$(id));

/** A moment for the sync round to have finished, without asserting on a clock. */
const settled = (): Promise<void> => new Promise((done) => setTimeout(done, 0));

describe('Drafts', () => {
  beforeEach(() => {
    TestBed.resetTestingModule();
    // A database of this test's own. IndexedDB is absent under jsdom anyway, and
    // [[Local]] answers `undefined` rather than throwing — which is the behaviour
    // every one of these relies on.
    TestBed.inject(Local).under(`t${Math.random().toString(36).slice(2)}`);
  });
  afterEach(async () => {
    await TestBed.inject(Drafts).close();
  });

  it('has nothing to say about a conversation nobody has written to', async () => {
    const { drafts } = harness();
    expect(await words(drafts, 'nobody')).toBe('');
  });

  it('keeps one conversation apart from another', async () => {
    const { drafts } = harness();
    await drafts.write('a', 'for a');
    await drafts.write('b', 'for b');
    expect(await words(drafts, 'a')).toBe('for a');
    expect(await words(drafts, 'b')).toBe('for b');
  });

  it('sends the whole document, which is what makes a push mergeable', async () => {
    const { drafts, sent } = harness();
    await drafts.write('a', 'half a thought');
    drafts.sync();
    await vi.waitFor(() => expect(sent.length).toBeGreaterThan(0));
    expect(sent[0]?.ulid).toBe('a');
    expect(reads(sent[0]?.update ?? '')).toBe('half a thought');
  });

  it('merges what the runner sends rather than replacing what is here', async () => {
    // The whole design in one assertion. The other device's words arrive
    // while this one holds its own, and BOTH survive. Under the old protocol this
    // was the moment somebody was asked to choose between them.
    const { drafts } = harness((url) =>
      url.includes('since=') ? holding('a', 'from the phone') : new Response('[]', { status: 200 }),
    );
    await drafts.write('a', 'from the mac');
    drafts.sync();
    await vi.waitFor(async () => {
      expect(await words(drafts, 'a')).toContain('from the phone');
    });
    expect(await words(drafts, 'a')).toContain('from the mac');
  });

  it('does not push back what the runner just sent it', async () => {
    // The echo loop, which is what a merging design gets wrong if it is
    // careless. A device that applies an arriving update and then counts it as
    // its own edit sends the runner's bytes back for ever.
    const { drafts, sent } = harness((url) =>
      url.includes('since=') ? holding('a', 'theirs') : new Response('[]', { status: 200 }),
    );
    drafts.sync();
    await vi.waitFor(async () => expect(await words(drafts, 'a')).toBe('theirs'));
    sent.length = 0;
    drafts.sync();
    await settled();
    expect(sent, 'the runner was sent its own document back').toEqual([]);
  });

  it('keeps a cleared draft as an empty one', async () => {
    const { drafts } = harness();
    await drafts.write('a', 'the message');
    await drafts.write('a', '');
    expect(await words(drafts, 'a')).toBe('');
  });

  it('says nothing about the words when the network fails', async () => {
    const { drafts } = harness(() => new Response('nope', { status: 500 }));
    const said: string[] = [];
    const warn = vi
      .spyOn(console, 'warn')
      .mockImplementation((...args: unknown[]) => said.push(args.join(' ')));
    await drafts.write('a', 'a private sentence');
    drafts.sync();
    await vi.waitFor(() => expect(said.length).toBeGreaterThan(0));
    warn.mockRestore();
    expect(said.join(' ')).not.toContain('a private sentence');
    // And the edit is not lost: it goes back on the pile for the next round.
    expect(await words(drafts, 'a')).toBe('a private sentence');
  });

  describe('the held picture', () => {
    it('comes back with a preview a reloaded page can show', async () => {
      const { drafts } = harness();
      await drafts.hold('a', PICTURE);
      const held = await firstValueFrom(drafts.picture$('a'));
      expect(held?.preview).toBe('data:image/png;base64,aGVsbG8=');
    });

    it('is put down when the composer lets go of it', async () => {
      const { drafts } = harness();
      await drafts.hold('a', PICTURE);
      await drafts.hold('a', undefined);
      expect(await firstValueFrom(drafts.picture$('a'))).toBeUndefined();
    });

    it('never crosses to the other device', async () => {
      // #89 settled this: hundreds of kilobytes, and no way to combine two.
      const { drafts, sent } = harness();
      await drafts.hold('a', PICTURE);
      drafts.sync();
      await settled();
      expect(
        sent.map((one) => one.update).join(''),
        'a picture was pushed to the runner',
      ).not.toContain('aGVsbG8');
    });
  });
});

describe('difference', () => {
  // A keystroke has to be ONE insert. Replacing the whole text instead
  // merges with a concurrent edit as two people retyping the sentence at once,
  // which is how a merging design can still lose words.
  it('takes a character added at the end as one insert', () => {
    expect(difference('hell', 'hello')).toEqual({ at: 4, removed: 0, added: 'o' });
  });

  it('takes a character removed at the end as one delete', () => {
    expect(difference('hello', 'hell')).toEqual({ at: 4, removed: 1, added: '' });
  });

  it('finds an edit in the middle without touching either end', () => {
    expect(difference('the cat sat', 'the dog sat')).toEqual({ at: 4, removed: 3, added: 'dog' });
  });

  it('says nothing changed when nothing did', () => {
    expect(difference('same', 'same')).toEqual({ at: 4, removed: 0, added: '' });
  });

  it('handles a box emptied outright', () => {
    expect(difference('gone', '')).toEqual({ at: 0, removed: 4, added: '' });
  });

  it('handles a box filled from empty', () => {
    expect(difference('', 'new')).toEqual({ at: 0, removed: 0, added: 'new' });
  });
});
