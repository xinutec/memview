import { Component, inject } from '@angular/core';
import { MAT_BOTTOM_SHEET_DATA } from '@angular/material/bottom-sheet';

import { Summary } from './models';
import { modeTitle } from './modes';
import { titleOf } from './naming';
import { fullness } from './tokens';

/**
 * One labelled fact about a session. `mono` marks identifiers — a path, a
 * model id — where a proportional face makes `l` and `1` the same shape.
 */
export interface Fact {
  readonly label: string;
  readonly value: string;
  readonly mono?: boolean;
  /** Where the value came from, or what it disagrees with, when that matters. */
  readonly note?: string;
}

/** What the sheet is opened with: the session, and what it is about. */
export interface Details {
  readonly session: Summary;
  readonly gist?: { readonly text: string; readonly at: number };
}

/**
 * Everything about a session that has nowhere else to be said. The header is
 * chosen for a glance, and a `title=` tooltip is no home for a fact — a phone
 * has no hover. Absent facts are left out rather
 * than shown blank: "not known yet" is not "missing".
 */
export function factsOf(session: Summary, gist?: Details['gist']): Fact[] {
  const facts: Fact[] = [];
  // First, and whole where the card clamps it to two lines.
  if (gist) {
    facts.push({
      label: 'about',
      value: gist.text,
      // Said in words: every other line here is read off a file or a process.
      note: `written by Haiku, ${when(gist.at)}`,
    });
  }
  facts.push({ label: 'where', value: session.dir, mono: true });
  // No "started with": for a resumed session it is the first prompt of the
  // seeded page, and for any long session it describes a job it has moved on
  // from. The id it is shipped under, not the header's name: `claude-opus-5` and
  // `claude-opus-5[1m]` are a million tokens apart.
  if (session.model) facts.push({ label: 'model', value: session.model, mono: true });
  const mode = modeTitle(session.mode);
  // The CLI's own term, matching `--permission-mode` in a terminal.
  if (mode) facts.push({ label: 'permission mode', value: mode });
  // What `--resume` takes — nowhere else in the console.
  facts.push({ label: 'session id', value: session.id, mono: true });
  // The session's other address, and the only one another session can reach it
  // by. Noted when it disagrees with the name on the card, which is what a
  // rename leaves behind until the conversation is next resumed — see
  // `Summary.peer_name`.
  if (session.peer_name) {
    const title = titleOf(session);
    facts.push({
      label: 'known to peers as',
      value: session.peer_name,
      mono: true,
      note:
        session.peer_name === title
          ? undefined
          : `this card says ${title}; a rename reaches here at the next resume`,
    });
  }
  // Absolute, where the list is relative.
  facts.push({ label: 'started', value: when(session.started * 1000) });
  if (session.touched) facts.push({ label: 'last active', value: when(session.touched) });
  // The two sizes a conversation has, next to each other because the gap is the
  // interesting quantity: `140k / 1M` under a 62 MB history has forgotten most
  // of itself.
  const full = fullness(session.context, session.window);
  if (full) facts.push({ label: 'context', value: full });
  // Looked up rather than scanned: four facts in the card's row wrap it.
  if (session.bytes) facts.push({ label: 'history', value: megabytes(session.bytes) });
  // Named for what it is, and not on the card: it is not a bill, since the
  // sessions run on the subscription. The usage strip answers "have I got room".
  if (session.cost_usd) {
    facts.push({ label: 'tokens at list price', value: `$${session.cost_usd.toFixed(2)}` });
  }
  // The CLI's own vocabulary, verbatim, and only when it is not the ordinary
  // answer.
  const limit = session.limit;
  if (limit && limit !== 'allowed') {
    facts.push({ label: 'rate limit', value: typeof limit === 'string' ? limit : limit.unknown });
  }
  return facts;
}

/** A moment, spelled out. The sheet is where somebody has stopped to look. */
function when(ms: number): string {
  return new Date(ms).toLocaleString();
}

/** Megabytes, floored at 1: a conversation with anything in it is not `0 MB`. */
function megabytes(bytes: number): string {
  return `${Math.max(1, Math.round(bytes / 1048576))} MB`;
}

/**
 * What this session is, in full. A bottom sheet: driven one-handed, it arrives
 * under the thumb that opened it.
 */
@Component({
  selector: 'app-session-sheet',
  templateUrl: './session-sheet.html',
  styleUrl: './session-sheet.scss',
})
export class SessionSheet {
  private readonly details = inject<Details>(MAT_BOTTOM_SHEET_DATA);
  /** The same name the button that opened this shows, and the same one the list
   *  card showed before that. See `naming.ts`. */
  protected readonly title = titleOf(this.details.session);
  protected readonly facts = factsOf(this.details.session, this.details.gist);
}
