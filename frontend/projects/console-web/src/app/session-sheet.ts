import { Component, inject } from '@angular/core';
import { MAT_BOTTOM_SHEET_DATA } from '@angular/material/bottom-sheet';

import { Summary } from './models';
import { modeTitle } from './modes';
import { titleOf } from './naming';
import { fullness } from './tokens';

/**
 * One labelled fact about a session.
 *
 * `mono` marks the values that are identifiers rather than prose — a path, a
 * model id, a session id. They are read a character at a time when they are read
 * at all, and a proportional face makes `l` and `1` the same shape.
 */
export interface Fact {
  readonly label: string;
  readonly value: string;
  readonly mono?: boolean;
}

/**
 * Everything about a session that has nowhere else to be said.
 *
 * ⚠ **The header is not a shorter version of this.** What the header shows is
 * chosen for a glance — is it working, how many exchanges, how full the context
 * — and three of the facts below were only ever reachable as a `title=`
 * tooltip: the full model id and the permission mode's name. **A phone has no
 * hover.** On the device this console exists for, those were written and
 * unreadable.
 *
 * Absent facts are left out rather than shown blank. A session the runner has
 * not finished reading has no name, no model and no mode, and a column of
 * em-dashes says "missing" where the truth is "not known yet".
 */
export function factsOf(session: Summary): Fact[] {
  const facts: Fact[] = [{ label: 'where', value: session.dir, mono: true }];
  // The instruction it was started with. On the list card and nowhere on the
  // session's own page — and it is what the whole conversation is about.
  if (session.asked) facts.push({ label: 'started with', value: session.asked });
  // The id it is shipped under, not the name the header shows: `claude-opus-5`
  // and `claude-opus-5[1m]` are one word apart on screen and a million tokens
  // apart in what they can hold.
  if (session.model) facts.push({ label: 'model', value: session.model, mono: true });
  const mode = modeTitle(session.mode);
  // The CLI's own term for this setting, so it matches what a person reading
  // `--permission-mode` in a terminal is looking at.
  if (mode) facts.push({ label: 'permission mode', value: mode });
  // What `--resume` takes. Nowhere else in the console at all, and it is the
  // one fact somebody needs when they want to pick this conversation up from a
  // terminal instead.
  facts.push({ label: 'session id', value: session.id, mono: true });
  // Absolute, where the list is relative. "9h ago" is the right answer to
  // "which of these is warm"; this is where you find out it has been running
  // since Tuesday.
  facts.push({ label: 'started', value: when(session.started * 1000) });
  if (session.touched) facts.push({ label: 'last active', value: when(session.touched) });
  // The two sizes a conversation has, and they belong next to each other
  // because the gap between them is the interesting quantity: the context is
  // what the model still has in front of it, the history is everything ever
  // said — compacted-away turns and whole tool results included. A session
  // reading `140k / 1M` under a 62 MB history has forgotten most of itself.
  const full = fullness(session.context, session.window);
  if (full) facts.push({ label: 'context', value: full });
  // Looked up rather than scanned, which is why it is here and not on the list
  // card: four facts in that row wrapped it onto a second line.
  if (session.bytes) facts.push({ label: 'history', value: megabytes(session.bytes) });
  // ⚠ **Named for what it is, which is why it is no longer a dollar sign on a
  // card.** It used to appear beside a session as soon as the account's own
  // verdict stopped being plain `allowed` — but that verdict is account-wide
  // while the display was per-session, so the figure landed on whichever
  // sessions happened to be talking when the API started warning: $422 against
  // `memview`, and nothing against `health`, which had spent $395.
  //
  // And it is not a bill in any case. These sessions inherit the CLI's
  // credentials and run on the subscription, so nothing is billed per token —
  // at the limit the work waits for the window to reset rather than being
  // charged for. What "have I got room" actually wants is the utilisation strip
  // on the front page, which is measured off the API's own headers rather than
  // inferred from a price list. So the number stays, where somebody who wants it
  // can look it up, saying what it is.
  if (session.cost_usd) {
    facts.push({ label: 'tokens at list price', value: `$${session.cost_usd.toFixed(2)}` });
  }
  // The CLI's own vocabulary — `allowed`, `allowed_warning`, `rejected` — kept
  // verbatim rather than reworded: these are the account's words, not ours.
  // Shown only when it is not the ordinary answer, because a line saying
  // `allowed` on every session is a line nobody reads.
  if (session.limit && session.limit !== 'allowed') {
    facts.push({ label: 'rate limit', value: session.limit });
  }
  return facts;
}

/** A moment, spelled out. The sheet is where somebody has stopped to look. */
function when(ms: number): string {
  return new Date(ms).toLocaleString();
}

/** Megabytes, which is the only sense of a transcript's size worth showing.
 *
 *  Floored at 1: a conversation with anything in it at all is not `0 MB`, and a
 *  fraction of a megabyte is a precision nobody is reading this for. */
function megabytes(bytes: number): string {
  return `${Math.max(1, Math.round(bytes / 1048576))} MB`;
}

/**
 * What this session is, in full.
 *
 * A bottom sheet rather than a centred dialog: this console is driven
 * one-handed, and a sheet arrives under the thumb that opened it and leaves with
 * a downward swipe. It is also the pattern the rest of the fleet uses on phones.
 */
@Component({
  selector: 'app-session-sheet',
  templateUrl: './session-sheet.html',
  styleUrl: './session-sheet.scss',
})
export class SessionSheet {
  protected readonly session = inject<Summary>(MAT_BOTTOM_SHEET_DATA);
  /** The same name the button that opened this shows, and the same one the list
   *  card showed before that. See `naming.ts`. */
  protected readonly title = titleOf(this.session);
  protected readonly facts = factsOf(this.session);
}
