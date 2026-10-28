import { Injectable, inject } from '@angular/core';
import { Observable, Subject, filter, map, startWith, switchMap } from 'rxjs';
import * as Y from 'yjs';

import { reason } from './errors';
import { Local } from './local';
import { Telemetry } from './telemetry';
import type { Picture } from './picture';

/**
 * What has been written and not sent, per conversation, shared between devices.
 * A document, not a string: two devices typing produce one text holding both
 * edits, merged here and in the runner (`console/src/drafts.rs`). Nothing is sent
 * until a person presses send.
 */

/** The name of the shared text inside a draft. The runner uses the same one. */
const TEXT = 'text';

/** How often to ask the runner for what it has. */
const PULL_EVERY_MS = 5000;

/**
 * How long an edit waits before it is sent. Not the poll interval: five seconds
 * on one device with nothing on the other reads as a failed merge. Long enough
 * that a sentence typed straight through is one push.
 */
const SEND_AFTER_MS = 300;

/** A held picture as stored: no preview, since an object URL dies with its document. */
interface HeldPicture {
  data: string;
  mediaType: string;
  width: number;
  height: number;
  bytes: number;
}

/** A stored picture, checked rather than cast: storage outlives the build that wrote it. */
function revived(value: unknown): Picture | undefined {
  if (typeof value !== 'object' || value === null) return undefined;
  const held: Partial<HeldPicture> = value;
  if (typeof held.data !== 'string' || typeof held.mediaType !== 'string') return undefined;
  if (typeof held.width !== 'number' || typeof held.height !== 'number') return undefined;
  if (typeof held.bytes !== 'number') return undefined;
  return {
    data: held.data,
    mediaType: held.mediaType,
    width: held.width,
    height: held.height,
    bytes: held.bytes,
    // A data URL rather than an object URL: nothing to revoke, and it cannot be
    // dead on arrival like a `blob:` from a previous document.
    preview: `data:${held.mediaType};base64,${held.data}`,
  };
}

/** The stored pictures; a single one is what the build before several wrote. */
function revivedAll(value: unknown): Picture[] {
  const all: unknown[] = Array.isArray(value) ? value : [value];
  return all.map(revived).filter((it) => it !== undefined);
}

/** One conversation's document and the machinery keeping it. */
interface Kept {
  readonly doc: Y.Doc;
  readonly text: Y.Text;
  /** Resolves once storage has given back what this device already had. */
  readonly loaded: Promise<unknown>;
}

function toBase64(bytes: Uint8Array): string {
  let binary = '';
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

function fromBase64(text: string): Uint8Array | undefined {
  try {
    const binary = atob(text);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    return bytes;
  } catch {
    return undefined;
  }
}

@Injectable({ providedIn: 'root' })
export class Drafts {
  private telemetry = inject(Telemetry);
  private store = inject(Local);
  private readonly kept = new Map<string, Kept>();
  private timer?: ReturnType<typeof setInterval>;
  private since = 0;
  /** Conversations whose document has changed here since the last push. */
  private readonly dirty = new Set<string>();
  private sending?: Promise<void>;
  private soon?: ReturnType<typeof setTimeout>;
  /** A conversation whose picture changed, so its stream re-reads. */
  private readonly pictures = new Subject<string | null>();
  private get: typeof fetch = (...args) => fetch(...args);

  /** Point this at a different runner, before anything else. Production never does. */
  // dev-lint: allow-ts-test-only the specs point it at a fake runner; production uses fetch
  configure(get: typeof fetch): void {
    this.get = get;
  }

  /** The words this conversation is holding, and every change to them. */
  text$(id: string): Observable<string> {
    return new Observable<string>((to) => {
      let stop: (() => void) | undefined;
      void this.open(id).then((kept) => {
        const say = (): void => to.next(kept.text.toJSON());
        say();
        kept.doc.on('update', say);
        stop = () => kept.doc.off('update', say);
      });
      return () => stop?.();
    });
  }

  /** The words this conversation is holding now. */
  async read(id: string): Promise<string> {
    return (await this.open(id)).text.toJSON();
  }

  /** Put `text` in the box, as a diff against what is there — see [[difference]]. */
  async write(id: string, text: string): Promise<void> {
    const kept = await this.open(id);
    const had = kept.text.toJSON();
    if (had === text) return;
    const { at, removed, added } = difference(had, text);
    kept.doc.transact(() => {
      if (removed > 0) kept.text.delete(at, removed);
      if (added) kept.text.insert(at, added);
    }, 'here');
  }

  /** Hold pictures for the next message; none puts them down. */
  async hold(id: string, pictures: readonly Picture[]): Promise<void> {
    if (!pictures.length) {
      await this.store.delete(`picture-${id}`);
      this.pictures.next(id);
      return;
    }
    const kept: HeldPicture[] = pictures.map((picture) => ({
      data: picture.data,
      mediaType: picture.mediaType,
      width: picture.width,
      height: picture.height,
      bytes: picture.bytes,
    }));
    await this.store.set(`picture-${id}`, kept);
    this.pictures.next(id);
  }

  /**
   * The pictures waiting to go with the next message. Local only: a picture is
   * hundreds of kilobytes and two cannot be combined.
   */
  pictures$(id: string): Observable<Picture[]> {
    return this.pictures.pipe(
      filter((changed) => changed === id || changed === null),
      startWith(id),
      switchMap(() => this.store.get(`picture-${id}`)),
      map(revivedAll),
    );
  }

  /** Ask the runner for anything new, and send anything of ours it lacks. */
  sync(): void {
    this.timer ??= setInterval(() => void this.exchange(), PULL_EVERY_MS);
    void this.exchange();
  }

  /** Send what has changed here, shortly — see [SEND_AFTER_MS]. */
  private push(): void {
    if (this.soon !== undefined) return;
    this.soon = setTimeout(() => {
      this.soon = undefined;
      void this.exchange();
    }, SEND_AFTER_MS);
  }

  /** Stop syncing and let go of every document. */
  async close(): Promise<void> {
    clearInterval(this.timer);
    clearTimeout(this.soon);
    this.soon = undefined;
    this.timer = undefined;
    for (const kept of this.kept.values()) kept.doc.destroy();
    this.kept.clear();
    await Promise.resolve();
  }

  /**
   * One round: push what changed here, then take what the runner has past our
   * cursor. Both directions merge bytes into a document, so no order goes wrong.
   */
  private async exchange(): Promise<void> {
    // One round at a time: a repeated push is harmless but costs a phone's bytes.
    this.sending ??= this.round().finally(() => {
      this.sending = undefined;
    });
    await this.sending;
  }

  private async round(): Promise<void> {
    const mine = [...this.dirty];
    this.dirty.clear();
    try {
      if (mine.length) {
        const entries = await Promise.all(
          mine.map(async (id) => ({
            ulid: id,
            update: toBase64(Y.encodeStateAsUpdate((await this.open(id)).doc)),
            at: Date.now(),
          })),
        );
        const answer = await this.get('/api/sync/drafts', {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify(entries),
        });
        if (!answer.ok) throw new Error(`draft push failed: ${answer.status}`);
        await this.take(await answer.json());
      }
      const res = await this.get(`/api/sync/drafts?since=${this.since}`);
      if (!res.ok) throw new Error(`draft pull failed: ${res.status}`);
      const body: unknown = await res.json();
      if (typeof body !== 'object' || body === null) return;
      if ('documents' in body) await this.take(body.documents);
      // A malformed cursor is left alone: as zero it would re-deliver the whole
      // store every five seconds.
      if ('checkpoint' in body) {
        const { checkpoint } = body;
        if (typeof checkpoint === 'object' && checkpoint !== null && 'rev' in checkpoint) {
          const { rev } = checkpoint;
          if (typeof rev === 'number') this.since = Math.max(this.since, rev);
        }
      }
    } catch (err: unknown) {
      // Said out loud: a silent failure looks like a quiet draft. Through
      // [[reason]], since `String(err)` gives `[object Object]`.
      const said = reason(err);
      console.warn('draft sync:', said);
      this.telemetry.note('draft-sync', said);
      // Unsent work goes back on the pile, or an edit made offline is never sent.
      for (const id of mine) this.dirty.add(id);
    }
  }

  /** Merge whatever the runner sent into the documents here. */
  private async take(documents: unknown): Promise<void> {
    if (!Array.isArray(documents)) return;
    // `unknown[]`, not `isArray`'s `any[]`: every field is checked, not trusted.
    const rows: readonly unknown[] = documents;
    for (const row of rows) {
      if (typeof row !== 'object' || row === null) continue;
      if (!('ulid' in row) || typeof row.ulid !== 'string') continue;
      if (!('update' in row) || typeof row.update !== 'string') continue;
      const update = fromBase64(row.update);
      if (!update) continue;
      const kept = await this.open(row.ulid);
      // Tagged `theirs`, which keeps it out of [dirty]: the runner's own bytes are
      // not echoed back to it.
      Y.applyUpdate(kept.doc, update, 'theirs');
    }
  }

  /** This conversation's document, restored from this device's storage. */
  private async open(id: string): Promise<Kept> {
    const already = this.kept.get(id);
    if (already) {
      await already.loaded;
      return already;
    }
    const doc = new Y.Doc();
    const kept: Kept = { doc, text: doc.getText(TEXT), loaded: this.restore(id, doc) };
    doc.on('update', (_update: Uint8Array, origin: unknown) => {
      // Anything written here is ours to send; anything merged in came from there.
      if (origin !== 'theirs') {
        this.dirty.add(id);
        this.push();
      }
      // Kept whole: a log of updates would grow as long as somebody typed.
      void this.store.set(`draft-${id}`, Y.encodeStateAsUpdate(doc));
    });
    this.kept.set(id, kept);
    await kept.loaded;
    return kept;
  }

  /** Put back what this device had, if anything, before anybody reads the box. */
  private async restore(id: string, doc: Y.Doc): Promise<void> {
    const held = await this.store.get(`draft-${id}`);
    if (held instanceof Uint8Array && held.length) Y.applyUpdate(doc, held, 'theirs');
  }
}

/**
 * The one span that changed between two strings: where, how much to remove, and
 * what to put there. A keystroke must be one insert of one character: replacing
 * the whole sentence merges with a concurrent edit as two people retyping at
 * once, which loses words. A shared prefix and suffix is enough for a person
 * typing.
 */
export function difference(
  had: string,
  wants: string,
): { at: number; removed: number; added: string } {
  let head = 0;
  while (head < had.length && head < wants.length && had[head] === wants[head]) head++;
  let tail = 0;
  while (
    tail < had.length - head &&
    tail < wants.length - head &&
    had[had.length - 1 - tail] === wants[wants.length - 1 - tail]
  ) {
    tail++;
  }
  return {
    at: head,
    removed: had.length - head - tail,
    added: wants.slice(head, wants.length - tail),
  };
}
