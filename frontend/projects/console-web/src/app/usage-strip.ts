import { Component, computed, input } from '@angular/core';
import { MatProgressBarModule } from '@angular/material/progress-bar';

import { Usage, Window } from './models';

/** One window as the strip draws it. */
interface Bar {
  label: string;
  pct: number;
  /** How long until it turns over. Absent once it already has — see [[Window]]. */
  left?: string;
  /** Near the ceiling, where the number stops being background information. */
  high: boolean;
  /**
   * How far through the window the clock is, 0–100 — or absent when it cannot
   * be known.
   *
   * ⚠ **Read at the SAME instant as `pct`, and that is the whole point.**
   * `resets_in_ms` and `pct` come from one reading, so comparing them is
   * coherent; recomputing "now" from the phone's clock would compare a fresh
   * time against a stale spend and drift besides — the thing [[Window]] is
   * written to prevent.
   */
  elapsed?: number;
  /**
   * Where the day boundaries fall, 0–100, for a window measured in days.
   *
   * Empty for the five-hour window: ticks are for judging pace across a week,
   * and five hours has no unit a person tracks.
   */
  days: number[];
}

/** Past this, a window is worth noticing rather than merely knowing. */
const LOUD = 80;

const HOUR = 3600_000;
const DAY = 24 * HOUR;

/**
 * How long each window runs.
 *
 * ⚠ **Carried as data, not read off the label.** `label === 'Week'` would make a
 * display string load-bearing, so renaming the row would silently drop its
 * markers — the same conflation the parse sheet had between a chip and its
 * style key.
 */
const FIVE_HOURS = 5 * HOUR;
const WEEK = 7 * DAY;

/**
 * What the subscription has spent, above the list of sessions.
 *
 * ⚠ **A percentage without its window is not a smaller number, it is no
 * number.** The reading comes from the home dashboard, which gets it from
 * Claude Code's status line — and a status line belongs to a terminal, so the
 * console's own headless sessions never refresh it. Readings hours old are
 * ordinary here, and a five-hour window that has turned over since leaves a
 * figure describing a window that no longer exists. Those are drawn as *no
 * reading*, not as the number they used to be.
 *
 * The age is on screen for the same reason, rather than in a tooltip a phone
 * cannot reach: this is a number somebody is about to make a decision with.
 */
@Component({
  selector: 'app-usage-strip',
  templateUrl: './usage-strip.html',
  styleUrl: './usage-strip.scss',
  imports: [MatProgressBarModule],
})
export class UsageStrip {
  /** The reading, or nothing — in which case the strip is not on screen. */
  readonly usage = input<Usage | undefined>(undefined);

  protected readonly bars = computed<Bar[]>(() => {
    const usage = this.usage();
    if (!usage) return [];
    return [
      // ⚠ A window the runner has heard nothing about gets no row at all —
      // absent is not the same as reset, and neither is the same as zero. See
      // [[Usage]].
      ...(usage.five_hour ? [bar('5 hours', usage.five_hour, FIVE_HOURS)] : []),
      // "Week", not "7 days": it is what the reading is called everywhere else.
      ...(usage.seven_day ? [bar('Week', usage.seven_day, WEEK)] : []),
      // A model's own weekly allowance, labelled with the model and nothing
      // else: the runner sends only the scopes the account actually has, and
      // the label is its name rather than a word this file chose for it.
      // A model's allowance runs the same week, so it gets the same markers.
      ...(usage.models ?? []).map((scope) => bar(scope.model, scope, WEEK)),
    ];
  });

  /** How old the reading is, in the same words the session list uses. */
  protected readonly taken = computed(() => {
    const usage = this.usage();
    return usage ? since(usage.age_ms) : '';
  });
}

function bar(label: string, window: Window, spanMs: number): Bar {
  const left = window.resets_in_ms;
  return {
    label,
    pct: Math.round(window.pct),
    left: left === undefined ? undefined : span(left),
    high: left !== undefined && window.pct >= LOUD,
    // ⚠ **Clamped, because a reading can outlive its own window.** The runner
    // reports what it last saw; a `resets_in_ms` larger than the window (a
    // reading taken just after a turnover) would otherwise place the marker off
    // the bar, and a marker off the bar is worse than none.
    elapsed: left === undefined ? undefined : clamp(((spanMs - left) / spanMs) * 100),
    days: spanMs >= 2 * DAY ? boundaries(spanMs) : [],
  };
}

/** Day boundaries inside a window, as percentages. Ends excluded — the bar's
 *  own edges already mark those. */
function boundaries(spanMs: number): number[] {
  const days = Math.round(spanMs / DAY);
  return Array.from({ length: days - 1 }, (_, i) => ((i + 1) / days) * 100);
}

function clamp(pct: number): number {
  return Math.min(100, Math.max(0, pct));
}

/** A duration, to the coarsest unit that still says something. */
function span(ms: number): string {
  const minutes = Math.max(1, Math.round(ms / 60000));
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h`;
  return `${Math.round(hours / 24)}d`;
}

/** The same duration, said backwards. */
function since(ms: number): string {
  return ms < 60000 ? 'just now' : `${span(ms)} ago`;
}
