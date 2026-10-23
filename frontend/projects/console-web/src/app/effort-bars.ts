import { Component, computed, input } from '@angular/core';

import { effortBars, effortName } from './model';
import { Effort } from './models';

/**
 * An effort as five rising bars beside its model: a level in the width of a
 * glyph, where `xhigh` in words costs as much as the model's name. A level not on
 * the scale is shown as its word; a model with no effort setting, as nothing.
 */
@Component({
  selector: 'app-effort-bars',
  templateUrl: './effort-bars.html',
  styleUrl: './effort-bars.scss',
})
export class EffortBars {
  readonly effort = input<Effort>();
  readonly filled = computed(() => effortBars(this.effort()));
  readonly name = computed(() => effortName(this.effort()));
  readonly places = [1, 2, 3, 4, 5];
}
