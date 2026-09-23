import { Component, computed, input } from '@angular/core';
import { MatProgressBarModule } from '@angular/material/progress-bar';

import { Reading, Window } from './models';

/** One window as the strip draws it. */
interface Bar {
  label: string;
  pct: number;
  /** How long until it turns over. Absent once it already has — see [[Window]]. */
  left?: string;
  /** Near the ceiling and spent faster than the clock: on course to run out before the reset. */
  high: boolean;
  /**
   * How far through the window the clock is, 0–100 — or absent. Read at the SAME
   * instant as `pct`: both come from one reading, and the phone's clock would
   * compare a fresh time against a stale spend.
   */
  elapsed?: number;
  /**
   * Where the day boundaries fall, 0–100, for a window measured in days. Empty
   * for the five-hour window.
   */
  days: number[];
}

/** Past this, a window is worth noticing rather than merely knowing. */
const LOUD = 80;

const HOUR = 3600_000;
const DAY = 24 * HOUR;

/**
 * How long each window runs — carried as data, not read off the label, so
 * renaming the row cannot drop its markers.
 */
const FIVE_HOURS = 5 * HOUR;
const WEEK = 7 * DAY;

/**
 * What the subscription has spent, above the list of sessions. A percentage
 * without its window is no number: readings hours old are ordinary, and a
 * window that has turned over since is drawn as *no reading*. The age is on
 * screen, not in a tooltip a phone cannot reach.
 */
@Component({
  selector: 'app-usage-strip',
  templateUrl: './usage-strip.html',
  styleUrl: './usage-strip.scss',
  imports: [MatProgressBarModule],
})
export class UsageStrip {
  /** The reading, or nothing — in which case the strip is not on screen. */
  readonly usage = input<Reading | undefined>(undefined);

  protected readonly bars = computed<Bar[]>(() => {
    const usage = this.usage();
    if (!usage) return [];
    return [
      // A window the runner has heard nothing about gets no row: absent is not reset,
      // and neither is zero. See [[Reading]].
      ...(usage.five_hour ? [bar('5 hours', usage.five_hour, FIVE_HOURS)] : []),
      // "Week", not "7 days": it is what the reading is called everywhere else.
      ...(usage.seven_day ? [bar('Week', usage.seven_day, WEEK)] : []),
      // A model's own weekly allowance, labelled with its name and nothing else; it
      // runs the same week, so it gets the same markers.
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
  // Clamped, because a reading can outlive its own window and a marker off the
  // bar is worse than none.
  const elapsed = left === undefined ? undefined : clamp(((spanMs - left) / spanMs) * 100);
  return {
    label,
    pct: Math.round(window.pct),
    left: left === undefined ? undefined : span(left),
    high: elapsed !== undefined && window.pct >= LOUD && window.pct > elapsed,
    elapsed,
    days: spanMs >= 2 * DAY ? boundaries(spanMs) : [],
  };
}

/** Day boundaries inside a window, as percentages, ends excluded. */
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
