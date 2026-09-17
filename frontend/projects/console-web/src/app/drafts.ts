import { Injectable, inject } from '@angular/core';
import { from, map, switchMap, type Observable } from 'rxjs';

import { ConsoleDb, type DraftDoc } from './console-db';
import { Telemetry } from './telemetry';
import type { Picture } from './picture';

/** What to do with a [[Clash]]. `both` keeps each text whole, in the order named. */
export type Resolution = 'mine' | 'theirs' | 'mine-first' | 'theirs-first';

export type { Clash } from './console-db';

/** A stored picture. Checked, not cast: storage outlives the build that wrote it. */
function revived(value: unknown): Picture | undefined {
  if (typeof value !== 'object' || value === null) return undefined;
  const held = value as Partial<HeldPicture>;
  if (typeof held.data !== 'string' || typeof held.mediaType !== 'string') return undefined;
  if (typeof held.width !== 'number' || typeof held.height !== 'number') return undefined;
  if (typeof held.bytes !== 'number') return undefined;
  return {
    data: held.data,
    mediaType: held.mediaType,
    width: held.width,
    height: held.height,
    bytes: held.bytes,
    // A data URL rather than an object URL: nothing to revoke, and it cannot be dead
    // on arrival like a `blob:` from a previous document.
    preview: `data:${held.mediaType};base64,${held.data}`,
  };
}

/**
 * A held picture as stored — no preview, since an object URL dies with the
 * document that made it.
 */
interface HeldPicture {
  data: string;
  mediaType: string;
  width: number;
  height: number;
  bytes: number;
}

/**
 * What has been written and not sent, per conversation, shared between devices.
 *
 * The collection is the only copy, and writes are not debounced: this device's
 * own write must come back as the value it already holds, so anything differing
 * is from elsewhere. Replication coalesces pushes itself.
 *
 * A draft is a RECORD, not an instruction; nothing leaves until a person presses
 * send. memview#90 refused the queued send for that reason.
 */
@Injectable({ providedIn: 'root' })
export class Drafts {
  private db = inject(ConsoleDb);
  private telemetry = inject(Telemetry);

  /**
   * Both texts, when this device and the runner have each moved since they
   * agreed. Raised by RxDB's conflict handler — see [[draftConflicts]].
   */
  readonly clash = this.db.clash;

  /**
   * The words this conversation is holding. `undefined` until the collection
   * answers — not the same as "nothing written", or a box being typed in blanks
   * while the database opens.
   */
  text$(id: string): Observable<string | undefined> {
    return this.doc$(id).pipe(map((doc) => doc?.text ?? ''));
  }

  /**
   * The picture waiting to go with the next message. A LOCAL document, so
   * replication cannot carry it — #89 settled that a picture does not cross devices.
   */
  picture$(id: string): Observable<Picture | undefined> {
    return from(this.db.collection()).pipe(
      switchMap((collection) => collection.getLocal$<HeldPicture>(`picture-${id}`)),
      // Checked, not asserted: what comes back may have been written by a build two
      // versions gone, and a bad cast lands far from here as `data:undefined;base64,…`.
      map((held) => revived(held?.toJSON().data)),
    );
  }

  /**
   * Record what the composer holds now — including nothing, which is what a send
   * leaves behind. Clearing writes a tombstone rather than removing the row, and a
   * conversation nobody has typed in gets no row; both are pinned by tests.
   */
  async write(id: string, text: string): Promise<void> {
    const collection = await this.db.collection();
    const existing = await collection.findOne(id).exec();
    if (!existing) {
      if (!text) return;
      await collection.insert({ ulid: id, text, at: Date.now(), rev: 0, _deleted: false });
      return;
    }
    // Incremental, so a keystroke applies to the document as it is now. `rev` is
    // the runner's to set.
    await existing.incrementalPatch({ text, at: Date.now() });
  }

  /** Hold a picture for the next message, or put it down. */
  async hold(id: string, picture: Picture | undefined): Promise<void> {
    const collection = await this.db.collection();
    const key = `picture-${id}`;
    const held = await collection.getLocal<HeldPicture>(key);
    if (!picture) {
      await held?.remove();
      return;
    }
    const kept: HeldPicture = {
      data: picture.data,
      mediaType: picture.mediaType,
      width: picture.width,
      height: picture.height,
      bytes: picture.bytes,
    };
    await collection.upsertLocal(key, kept);
  }

  /** Ask the runner now — see [[ConsoleDb.resync]] for the two moments. */
  sync(): void {
    this.db.resync();
  }

  /**
   * Settle a clash: put the chosen words in, and let them replicate. Written on
   * top of THEIRS, since the conflict handler gave the master to the runner, and
   * writing from the losing state would bounce off the same refusal.
   */
  async resolve(id: string, theirs: DraftDoc, how: Resolution): Promise<void> {
    const mine = this.clash()?.mine ?? '';
    const text =
      how === 'mine'
        ? mine
        : how === 'theirs'
          ? theirs.text
          : how === 'mine-first'
            ? `${mine}\n\n${theirs.text}`
            : `${theirs.text}\n\n${mine}`;
    // Which button and the three lengths — never the words.
    const said =
      `draft clash on ${id.slice(0, 8)} settled as ${how}: ` +
      `${mine.length} here, ${theirs.text.length} there, ${text.length} kept`;
    console.warn(said);
    this.telemetry.note('draft-settled', said);
    this.db.settled();
    await this.write(id, text);
  }

  /** The document, as a stream. */
  private doc$(id: string): Observable<DraftDoc | null> {
    return from(this.db.collection()).pipe(
      switchMap((collection) => collection.findOne(id).$),
      map((doc) => (doc ? (doc.toJSON() as DraftDoc) : null)),
    );
  }
}
