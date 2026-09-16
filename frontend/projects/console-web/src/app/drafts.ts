import { Injectable, inject, signal } from '@angular/core';

import { DraftsDb, type DraftDoc } from './drafts-db';
import type { Picture } from './picture';

/**
 * A stored draft picture, checked rather than asserted.
 *
 * ⚠ **Storage outlives every deploy that touched this phone.** What comes back
 * may have been written by a version of this app that is two builds gone, and a
 * `JSON.parse(…) as Picture` would be a claim about code that no longer runs —
 * with the damage landing far from here, in an upload of `undefined` or a
 * preview reading `data:undefined;base64,undefined`. Each field is checked, and
 * anything else is simply not a draft.
 */
function revived(value: unknown): Picture | undefined {
  if (typeof value !== 'object' || value === null) return undefined;
  if (!('data' in value) || typeof value.data !== 'string') return undefined;
  if (!('mediaType' in value) || typeof value.mediaType !== 'string') return undefined;
  if (!('width' in value) || typeof value.width !== 'number') return undefined;
  if (!('height' in value) || typeof value.height !== 'number') return undefined;
  if (!('bytes' in value) || typeof value.bytes !== 'number') return undefined;
  const { data, mediaType, width, height, bytes } = value;
  return {
    data,
    mediaType,
    width,
    height,
    bytes,
    // A data URL rather than an object URL: nothing to revoke, and it cannot be
    // dead on arrival the way a `blob:` from a previous document is.
    preview: `data:${mediaType};base64,${data}`,
  };
}

export type { Clash } from './drafts-db';

/** What to do with a [[Clash]]. `both` keeps each text whole, in the order named. */
export type Resolution = 'mine' | 'theirs' | 'mine-first' | 'theirs-first';

/**
 * How long typing must pause before the words go to the collection.
 *
 * ⚠ **Not tidying — RxDB pushes per local WRITE.** `put` is called per
 * keystroke, so writing straight through would be a request per character.
 * Debouncing here rather than inside the replication keeps the collection's
 * contents true at every moment a person could look at them.
 */
const WRITE_AFTER_MS = 800;

/**
 * What has been written and not sent, kept per session and shared between
 * devices.
 *
 * ⚠ **This is a MIRROR and a debounce in front of [[DraftsDb]], nothing more.**
 * Everything that used to be hand-written here — the revision each session was
 * in step with, the last text agreed with the runner, the retry, the
 * three-way reconcile — is gone. That bookkeeping produced four bugs in three
 * hours on 2026-09-15, none of them found by its tests, and every one of them
 * was a thing RxDB already does. What is left is the two jobs a library cannot
 * do for us.
 *
 * ⚠ **The first is that the composer must paint SYNCHRONOUSLY.** IndexedDB is
 * async, so seeding the box from the collection would flash an empty composer
 * on every open — the defect this file was written to fix. `localStorage` holds
 * a copy purely so `text()` can answer before first paint. It is never the
 * truth; the collection is.
 *
 * ⚠ **The second is the picture**, which does not replicate at all: it is
 * hundreds of kilobytes of base64 against a sentence's few hundred bytes, two
 * images have no meaningful combination, and the device that took one is
 * usually the device that wants it.
 *
 * ⚠ **A draft is a RECORD, not an instruction.** Nothing leaves until a person
 * presses send, which is what separates this from the queued send memview #90
 * refused — do not read draft replication as permission to build that.
 */
@Injectable({ providedIn: 'root' })
export class Drafts {
  private static readonly PREFIX = 'console.draft.';
  private db = inject(DraftsDb);

  /** Both texts, when this device and the runner have each moved since they
   *  agreed. Raised by RxDB's conflict handler — see [[draftConflicts]]. */
  readonly clash = this.db.clash;

  /**
   * Text that arrived from the other device, for a session.
   *
   * ⚠ **Not an offer to be accepted — it has already happened.** Where both
   * sides had written, the conflict handler raises a [[Clash]] instead and
   * nothing is overwritten. So anything reaching here is a change this device
   * had nothing at stake in, and the composer's job is to show it rather than
   * ask about it. That is also what empties the phone's box when the Mac sends.
   */
  readonly landed = signal<{ id: string; text: string } | undefined>(undefined);

  /** The truth for this page before the collection can answer. See the note on
   *  synchronous painting above. */
  private readonly held = new Map<string, { text: string; picture?: Picture }>();
  private readonly timers = new Map<string, ReturnType<typeof setTimeout>>();

  /**
   * What the composer and the collection last AGREED on, per session. Set three
   * ways: the seed when a session is first read, words that arrive from the
   * other device, and this device's own writes.
   *
   * ⚠ **This is the difference between somebody typing and the box echoing, and
   * there is no other way to tell.** The composer records itself back on every
   * change, including the change of being filled in — so `put(id, '')` arrives
   * both when a conversation is merely OPENED and when a message has just been
   * SENT, and those two must do opposite things. A `put` carrying exactly what
   * was last agreed is the echo, and writes nothing.
   *
   * ⚠ **A session never read has been handed nothing, which counts as an empty
   * box** — otherwise the open echo slips through whenever the recording effect
   * runs before anything asked for the text.
   *
   * ⚠ **And it must move on every write, or clearing a draft BY HAND stops
   * working.** Type, then select-all and delete: without the write updating
   * this, the empty box matches the seed and reads as an echo, and the words
   * stay on the other device after somebody deliberately threw them away.
   *
   * This is the `synced` map the hand-rolled store kept, and deleting it as
   * "bookkeeping a library does" was wrong — a library cannot know which of two
   * identical calls came from a person.
   */
  private readonly given = new Map<string, string>();

  constructor() {
    void this.watch();
  }

  /** What was being typed, or an empty string. */
  text(id: string): string {
    return this.load(id).text;
  }

  /** The picture that was waiting to go with it, if there was one. */
  picture(id: string): Picture | undefined {
    return this.load(id).picture;
  }

  /** Ask the runner now — see [[DraftsDb.resync]] for the two moments. */
  sync(): void {
    this.db.resync();
  }

  /**
   * Record what the composer holds now — including nothing, which is what a
   * successful send leaves behind and is how a draft is forgotten.
   */
  put(id: string, text: string, picture: Picture | undefined): void {
    const echo = (this.given.get(id) ?? '') === text;
    this.held.set(id, { text, picture });
    this.write(`${id}.text`, text || undefined);
    // Stored without the preview: an object URL belongs to the document that
    // made it, so keeping one would store a string that is dead by the time
    // anything reads it. The bytes are here, and `load` builds a data URL.
    this.write(
      `${id}.picture`,
      picture &&
        JSON.stringify({
          data: picture.data,
          mediaType: picture.mediaType,
          width: picture.width,
          height: picture.height,
          bytes: picture.bytes,
        }),
    );
    // ⚠ **An echo is mirrored but never written** — see [[given]]. Without this
    // a device that only OPENED a conversation holding a draft wrote its own
    // empty composer over it and cleared the words on the other screen; found on
    // 2026-09-16 by two browsers against one runner, not by any test.
    if (!echo) this.schedule(id, text);
  }

  /**
   * Settle a clash: put the chosen words in, and let them replicate.
   *
   * ⚠ **Written from THEIRS, which is what makes the choice stick.** The
   * conflict handler gave the master to the runner, so the collection already
   * holds the other device's text; writing on top of that is an ordinary edit
   * from what is there. Pushing from the losing state instead would bounce off
   * the same refusal that raised the clash.
   */
  resolve(id: string, theirs: DraftDoc, how: Resolution): void {
    const mine = this.clash()?.mine ?? this.load(id).text;
    const text =
      how === 'mine'
        ? mine
        : how === 'theirs'
          ? theirs.text
          : how === 'mine-first'
            ? `${mine}\n\n${theirs.text}`
            : `${theirs.text}\n\n${mine}`;
    this.db.settled();
    this.given.delete(id);
    this.held.set(id, { text, picture: this.picture(id) });
    this.write(`${id}.text`, text || undefined);
    // At once rather than debounced: settling is a deliberate act, and the
    // person is watching the screen they did it on.
    void this.store(id, text);
  }

  /**
   * Follow the collection, so a change from the other device reaches the
   * composer while somebody is looking at it.
   *
   * ⚠ **Where a change CAME FROM is deliberately not asked.** RxDB writes a
   * replicated document into the same fork a local edit goes to, so telling the
   * two apart means reading its internals. Comparing against the mirror answers
   * the question that actually matters and cannot drift: a local edit updated
   * the mirror before it ever reached the collection, so its own echo finds the
   * mirror already equal and says nothing. Anything that differs came from
   * somewhere else.
   */
  private async watch(): Promise<void> {
    let collection;
    try {
      collection = await this.db.collection();
    } catch (err: unknown) {
      // ⚠ **A database that will not open must not take the composer with it.**
      // IndexedDB is refused outright in some private-browsing modes and can
      // fail on a quota or an old WebView, and none of that is a reason to stop
      // somebody typing: the mirror still answers, the words are still kept, and
      // what is lost is only the other device. Said out loud because from the
      // composer this looks exactly like a quiet tunnel.
      console.warn('drafts: no collection, so this device is on its own —', err);
      return;
    }
    collection.$.subscribe((event) => {
      const doc = event.documentData;
      const id = doc.ulid;
      const mine = this.held.get(id);
      if (mine?.text === doc.text) return;
      this.held.set(id, { text: doc.text, picture: mine?.picture ?? this.storedPicture(id) });
      this.given.set(id, doc.text);
      this.write(`${id}.text`, doc.text || undefined);
      this.landed.set({ id, text: doc.text });
    });
  }

  private schedule(id: string, text: string): void {
    clearTimeout(this.timers.get(id));
    this.timers.set(
      id,
      setTimeout(() => {
        this.timers.delete(id);
        void this.store(id, text);
      }, WRITE_AFTER_MS),
    );
  }

  /**
   * Put the words in the collection, which is what makes them replicate.
   *
   * ⚠ **`rev` is carried through untouched, never chosen here.** It is the
   * runner's pull cursor; a client that minted one would be inventing an
   * ordering the runner is the only authority on. An unchanged text writes
   * nothing at all — otherwise the echo of a pull would be pushed straight back.
   *
   * ⚠ **AND OPENING A SESSION IS NOT A STATEMENT ABOUT ITS DRAFT.** The
   * composer's recording effect runs once when a conversation opens, carrying
   * whatever is on screen — `''` where nothing has been typed here. Creating a
   * document for that makes a device that has only LOOKED the first writer of
   * the conversation, and the device that actually typed something is then
   * refused and shown a clash against nothing. That is memview#89's first live
   * bug; it came back on 2026-09-16 when the hand-rolled guard was deleted with
   * the rest of the bookkeeping, and was caught by driving two browsers at one
   * runner rather than by any test here.
   *
   * Clearing an EXISTING draft is a different thing and must still be written:
   * it is what a send leaves behind, and the tombstone is what stops the other
   * device pushing the message back.
   */
  private async store(id: string, text: string): Promise<void> {
    try {
      const collection = await this.db.collection();
      const existing = await collection.findOne(id).exec();
      if (!existing && !text) return;
      this.given.set(id, text);
      if (existing?.text === text) return;
      await collection.upsert({
        ulid: id,
        text,
        at: Date.now(),
        rev: existing?.rev ?? 0,
        _deleted: false,
      });
    } catch (err: unknown) {
      // The words are in the mirror and on screen; a write that cannot land is
      // what the next keystroke, the next foreground, and the replication's own
      // retry are all for. Said rather than swallowed — the phone's only window
      // is `adb logcat`.
      console.warn(`draft ${id.slice(0, 8)} did not reach the collection:`, err);
    }
  }

  private load(id: string): { text: string; picture?: Picture } {
    const known = this.held.get(id);
    if (known) return known;
    const draft = {
      text: localStorage.getItem(`${Drafts.PREFIX}${id}.text`) ?? '',
      picture: this.storedPicture(id),
    };
    this.held.set(id, draft);
    this.given.set(id, draft.text);
    return draft;
  }

  private storedPicture(id: string): Picture | undefined {
    const stored = localStorage.getItem(`${Drafts.PREFIX}${id}.picture`);
    if (!stored) return undefined;
    try {
      return revived(JSON.parse(stored));
    } catch {
      // Storage is shared with whatever else runs on this origin and a half
      // written value survives a kill. Losing a draft picture is a cost worth
      // paying; a session that will not open is not.
      return undefined;
    }
  }

  /**
   * Mirror one field, or remove it when there is nothing to keep.
   *
   * ⚠ **A full quota must not cost the words.** The picture is the only thing
   * here big enough to fill one — a scaled screenshot is a few hundred kilobytes
   * of base64 against a sentence's few hundred bytes — so a failed write is
   * swallowed per field rather than per draft. What it costs is the reload: the
   * picture is still in memory and still in the composer, and only a page that
   * comes back finds it gone.
   */
  private write(key: string, value: string | undefined): void {
    try {
      if (value === undefined) localStorage.removeItem(`${Drafts.PREFIX}${key}`);
      else localStorage.setItem(`${Drafts.PREFIX}${key}`, value);
    } catch {
      // Nothing to do and nothing to say: see above.
    }
  }
}
