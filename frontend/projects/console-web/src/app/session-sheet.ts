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
  /**
   * Where the value came from, when it matters: the one fact that is not a fact,
   * the sentence a model wrote.
   */
  readonly note?: string;
}

/** What the sheet is opened with: the session, and what it is about. */
export interface Details {
  readonly session: Summary;
  readonly gist?: { readonly text: string; readonly at: number };
}

/**
 * Everything about a session that has nowhere else to be said. The header is
 * chosen for a glance, and three of these facts were only reachable as a
 * `title=` tooltip — a phone has no hover. Absent facts are left out rather
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
  // Absolute, where the list is relative.
  facts.push({ label: 'started', value: when(session.started * 1000) });
  if (session.touched) facts.push({ label: 'last active', value: when(session.touched) });
  // The two sizes a conversation has, next to each other because the gap is the
  // interesting quantity: `140k / 1M` under a 62 MB history has forgotten most
  // of itself.
  const full = fullness(session.context, session.window);
  if (full) facts.push({ label: 'context', value: full });
  // Looked up rather than scanned: four facts in the card's row wrapped it.
  if (session.bytes) facts.push({ label: 'history', value: megabytes(session.bytes) });
  // Named for what it is. It was a dollar sign on a card, shown once the
  // account's verdict stopped being `allowed` — but that verdict is account-wide
  // and the display per-session, so $422 landed on `memview` and nothing on
  // `health`, which had spent $395. And it is not a bill: the sessions run on the
  // subscription. The utilisation strip is what "have I got room" wants.
  if (session.cost_usd) {
    facts.push({ label: 'tokens at list price', value: `$${session.cost_usd.toFixed(2)}` });
  }
  // The CLI's own vocabulary, verbatim, and only when it is not the ordinary
  // answer.
  if (session.limit && session.limit !== 'allowed') {
    facts.push({ label: 'rate limit', value: session.limit });
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
