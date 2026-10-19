import { Component, computed, input, model, output } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatIconModule } from '@angular/material/icon';

import { MOST, Picture, weight } from './picture';

/**
 * The box a message is written in, and the one button beside it. It owns no
 * state: the text is two-way so the view can seed it from a draft and empty it
 * on a send, and the picture stays in the parent, which scales and sends it.
 */
@Component({
  selector: 'app-composer',
  templateUrl: './composer.html',
  styleUrl: './composer.scss',
  imports: [FormsModule, MatButtonModule, MatIconModule],
})
export class Composer {
  /** What is written and not sent. */
  readonly text = model.required<string>();

  /** A send is in flight, so pressing again would send it twice. */
  readonly sending = input(false);

  /**
   * Pictures held for the next message, scaled and ready. In the parent because a
   * send carries them — and a picture with nothing said about it is a whole
   * message, so they are half of what decides whether there is anything to send.
   */
  readonly held = input<readonly Picture[]>([]);

  /** Why a chosen image was refused, when one was. */
  readonly trouble = input('');

  readonly send = output<void>();

  /**
   * Put a held picture down without sending it, by its place in [held]. Not
   * `drop`, a DOM event name `@angular-eslint/no-output-native` refuses: a
   * listener would fire on a drag.
   */
  readonly discard = output<number>();

  /** Images chosen from the picker, in the order chosen. */
  readonly chose = output<File[]>();

  protected readonly most = MOST;

  /** One picture's shape and weight, or how many and their weight together. */
  protected readonly about = computed(() => {
    const held = this.held();
    const bytes = held.reduce((sum, it) => sum + it.bytes, 0);
    const [one] = held;
    return held.length === 1 && one
      ? `${one.width}×${one.height} · ${weight(bytes)}`
      : `${held.length} pictures · ${weight(bytes)}`;
  });

  /** Something to send: words, or a picture on its own. */
  protected readonly ready = computed(() => !!this.text().trim() || this.held().length > 0);

  /** Cleared afterwards: a file input keeps its selection, so picking the same file twice fires no second `change`. */
  protected took(input: HTMLInputElement): void {
    const files = [...(input.files ?? [])];
    input.value = '';
    if (files.length) this.chose.emit(files);
  }
}
