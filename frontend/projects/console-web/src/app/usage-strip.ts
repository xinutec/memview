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
}

/** Past this, a window is worth noticing rather than merely knowing. */
const LOUD = 80;

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
      bar('5 hours', usage.five_hour),
      // "Week", not "7 days": it is what the reading is called everywhere else.
      bar('Week', usage.seven_day),
    ];
  });

  /** How old the reading is, in the same words the session list uses. */
  protected readonly taken = computed(() => {
    const usage = this.usage();
    return usage ? since(usage.age_ms) : '';
  });
}

function bar(label: string, window: Window): Bar {
  return {
    label,
    pct: Math.round(window.pct),
    left: window.resets_in_ms === undefined ? undefined : span(window.resets_in_ms),
    high: window.resets_in_ms !== undefined && window.pct >= LOUD,
  };
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
