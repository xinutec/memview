import { Injectable } from '@angular/core';

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

/**
 * What has been written and not sent, kept per session.
 *
 * Typed words and a chosen picture used to live on the session component, so
 * leaving the conversation destroyed them — and on a phone the words are the
 * expensive part and the picture cost a scale of a 4080×3072 photograph. Held
 * here instead, they outlive the page as well as the component: `Updates`
 * reloads when the app is put away, and a restored page reloads at once now, so
 * a draft that only lived in memory would be lost by the console's own
 * housekeeping.
 *
 * ⚠ **Keyed by session, not global.** Two conversations each hold their own
 * unsent message, which is the whole reason to leave one for the other.
 */
@Injectable({ providedIn: 'root' })
export class Drafts {
  private static readonly PREFIX = 'console.draft.';

  /**
   * The truth for this page, hydrated from storage the first time a session is
   * asked about.
   *
   * A cache and not an optimisation: a revived picture's preview is built here,
   * and rebuilding it per read would hand the template a different string every
   * change detection and reload the image each time.
   */
  private readonly held = new Map<string, { text: string; picture?: Picture }>();

  /** What was being typed, or an empty string. */
  text(id: string): string {
    return this.load(id).text;
  }

  /** The picture that was waiting to go with it, if there was one. */
  picture(id: string): Picture | undefined {
    return this.load(id).picture;
  }

  /**
   * Record what the composer holds now — including nothing, which is what a
   * successful send leaves behind and is how a draft is forgotten.
   */
  put(id: string, text: string, picture: Picture | undefined): void {
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
