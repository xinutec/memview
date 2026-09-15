import { Injectable, inject, signal } from '@angular/core';

import { ConsoleApi } from './console-api';
import type { StoredDraft } from './models';
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

/** Both texts, when two devices have written since they last agreed. */
export interface Clash {
  readonly id: string;
  readonly mine: string;
  readonly theirs: StoredDraft;
}

/** What to do with a [[Clash]]. `both` keeps each text whole, in the order named. */
export type Resolution = 'mine' | 'theirs' | 'mine-first' | 'theirs-first';

/** How long typing must pause before the words go to the runner. */
const PUSH_AFTER_MS = 800;

/**
 * What has been written and not sent, kept per session and shared between
 * devices.
 *
 * ⚠ **Local storage is what the composer reads, always.** Typing must not wait
 * for a round trip and must not stop when the tunnel does, so an edit lands here
 * first and goes to the runner afterwards. The runner is what makes the same
 * draft appear on the other device — see `console/src/drafts.rs`.
 *
 * ⚠ **Keyed by session, not global.** Two conversations each hold their own
 * unsent message, which is the whole reason to leave one for the other.
 *
 * ⚠ **A draft is a RECORD, not an instruction.** Nothing leaves until a person
 * presses send, which is what separates this from the queued send memview #90
 * refused — do not read draft replication as permission to build that.
 */
@Injectable({ providedIn: 'root' })
export class Drafts {
  private static readonly PREFIX = 'console.draft.';
  private api = inject(ConsoleApi);

  /**
   * The truth for this page, hydrated from storage the first time a session is
   * asked about.
   *
   * A cache and not an optimisation: a revived picture's preview is built here,
   * and rebuilding it per read would hand the template a different string every
   * change detection and reload the image each time.
   */
  private readonly held = new Map<string, { text: string; picture?: Picture }>();

  /** The runner revision each session's local text was last in step with. */
  private readonly revs = new Map<string, number>();

  private readonly timers = new Map<string, ReturnType<typeof setTimeout>>();

  /**
   * A draft the runner holds that this device has not adopted, and the clash
   * when it cannot be adopted silently. Signals rather than callbacks so the
   * view can draw them without this service knowing a view exists.
   */
  readonly incoming = signal<{ id: string; draft: StoredDraft } | undefined>(undefined);
  readonly clash = signal<Clash | undefined>(undefined);

  /** What was being typed, or an empty string. */
  text(id: string): string {
    return this.load(id).text;
  }

  /** The picture that was waiting to go with it, if there was one. */
  picture(id: string): Picture | undefined {
    return this.load(id).picture;
  }

  /**
   * Ask the runner what it holds for this session.
   *
   * ⚠ **Adopted only when this device has nothing unsent of its own.** Otherwise
   * opening a session on the phone would discard what was typed there in favour
   * of whatever the Mac last pushed. Where both hold text, it is a [[Clash]] and
   * a person decides.
   */
  open(id: string): void {
    this.api.draft(id).subscribe({
      next: (draft) => {
        if (!draft) return;
        const mine = this.load(id).text;
        if (mine === draft.text) {
          this.revs.set(id, draft.rev);
          return;
        }
        if (mine === '') {
          this.incoming.set({ id, draft });
          return;
        }
        this.clash.set({ id, mine, theirs: draft });
      },
      // Silent: the runner being unreachable is the case this store exists for,
      // and the local draft is already on screen.
      error: () => undefined,
    });
  }

  /** Take a draft the runner offered, once nothing local is at stake. */
  adopt(id: string, draft: StoredDraft): void {
    this.revs.set(id, draft.rev);
    this.put(id, draft.text, this.picture(id), { push: false });
    this.incoming.set(undefined);
  }

  /**
   * Settle a clash.
   *
   * Pushed from THEIRS' revision, which is what makes the chosen text win rather
   * than bounce off the same refusal that produced the clash.
   */
  resolve(id: string, theirs: StoredDraft, how: Resolution): void {
    const mine = this.load(id).text;
    const text =
      how === 'mine'
        ? mine
        : how === 'theirs'
          ? theirs.text
          : how === 'mine-first'
            ? `${mine}\n\n${theirs.text}`
            : `${theirs.text}\n\n${mine}`;
    this.revs.set(id, theirs.rev);
    this.clash.set(undefined);
    this.put(id, text, this.picture(id));
  }

  /**
   * Record what the composer holds now — including nothing, which is what a
   * successful send leaves behind and is how a draft is forgotten.
   *
   * The push is debounced because this is called per keystroke; a request per
   * character would be a request per character.
   */
  put(id: string, text: string, picture: Picture | undefined, opts = { push: true }): void {
    this.held.set(id, { text, picture });
    this.write(`${id}.text`, text || undefined);
    // Stored without the preview: an object URL belongs to the document that
    // made it, so keeping one would store a string that is dead by the time
    // anything reads it. The bytes are here, and `load` builds a data URL.
    //
    // ⚠ The picture is NOT pushed to the runner. It is hundreds of kilobytes of
    // base64 against a sentence's few hundred bytes, two images have no
    // meaningful combination, and the device that took one usually wants it.
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
    if (opts.push) this.schedule(id, text);
  }

  private schedule(id: string, text: string): void {
    clearTimeout(this.timers.get(id));
    this.timers.set(
      id,
      setTimeout(() => {
        this.timers.delete(id);
        this.push(id, text);
      }, PUSH_AFTER_MS),
    );
  }

  /** Send this device's words, and hand a refusal to the view as a clash. */
  private push(id: string, text: string): void {
    const from = this.revs.get(id);
    this.api.putDraft(id, { text, from, by: this.device() }).subscribe({
      next: (stored) => this.revs.set(id, stored.rev),
      error: (err: { status?: number; error?: unknown }) => {
        const theirs = err.status === 409 ? asDraft(err.error) : undefined;
        // Anything else is the tunnel being down, which is what local storage is
        // for: the words are safe and the next pause tries again.
        if (theirs) this.clash.set({ id, mine: text, theirs });
      },
    });
  }

  /**
   * What this device calls itself, so a clash can say whose text is whose.
   *
   * Minted once and kept: the name has to survive a reload or the same device
   * appears as a stranger to itself after every upgrade.
   */
  private device(): string {
    const key = `${Drafts.PREFIX}device`;
    const known = localStorage.getItem(key);
    if (known) return known;
    const name = /Android|iPhone|iPad/.test(navigator.userAgent) ? 'phone' : 'desktop';
    this.write('device', name);
    return name;
  }

  private load(id: string): { text: string; picture?: Picture } {
    const known = this.held.get(id);
    if (known) return known;
    const draft = {
      text: localStorage.getItem(`${Drafts.PREFIX}${id}.text`) ?? '',
      picture: this.storedPicture(id),
    };
    this.held.set(id, draft);
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

/**
 * The other device's draft out of a 409 body.
 *
 * ⚠ **Checked, not cast, for the reason [[revived]] gives one layer down.** This
 * one arrives from another machine, which may be running a different build, so
 * the wire deserves what storage already gets.
 */
function asDraft(value: unknown): StoredDraft | undefined {
  if (typeof value !== 'object' || value === null) return undefined;
  if (!('text' in value) || typeof value.text !== 'string') return undefined;
  if (!('rev' in value) || typeof value.rev !== 'number') return undefined;
  if (!('by' in value) || typeof value.by !== 'string') return undefined;
  if (!('at' in value) || typeof value.at !== 'number') return undefined;
  const { text, rev, by, at } = value;
  return { text, rev, by, at };
}
