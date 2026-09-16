import { Component, input, model, output } from '@angular/core';
import { TextFieldModule } from '@angular/cdk/text-field';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';

import { Picture, weight } from './picture';

/**
 * The box a message is written in, and the two buttons beside it.
 *
 * Split out of [[SessionView]] because that component's stylesheet reached the
 * ceiling that fails the build. It owns no state: the text is two-way so the
 * view can seed it from a draft and empty it on a send, and the picture stays in
 * the parent, which is what holds it, scales it and sends it.
 */
@Component({
  selector: 'app-composer',
  templateUrl: './composer.html',
  styleUrl: './composer.scss',
  imports: [
    FormsModule,
    MatButtonModule,
    MatIconModule,
    MatFormFieldModule,
    MatInputModule,
    TextFieldModule,
  ],
})
export class Composer {
  /** What is written and not sent. */
  readonly text = model.required<string>();

  /** A send is in flight, so pressing again would send it twice. */
  readonly sending = input(false);

  /**
   * A picture held for the next message, scaled and ready. Passed rather than
   * owned: the view holds it because a send carries it, and a picture with
   * nothing said about it is a whole message — so it is half of what decides
   * whether there is anything to send.
   */
  readonly held = input<Picture | undefined>(undefined);

  /** Why a chosen image was refused, when one was. */
  readonly trouble = input('');

  readonly send = output<void>();

  /**
   * Put the held picture down without sending it.
   *
   * Not `drop`, which the view calls it: that is a DOM event name, and
   * `@angular-eslint/no-output-native` refuses it on an output for good reason —
   * a listener would fire on a drag as well.
   */
  readonly discard = output<void>();

  /** An image chosen from the picker, already the one file that matters. */
  readonly chose = output<File>();

  /**
   * ⚠ **The input is cleared afterwards, and it matters.** A file input holds
   * its selection, so choosing the same screenshot twice in a row fires no
   * `change` event the second time and the picker simply appears to do nothing.
   */
  protected readonly weight = weight;

  protected took(input: HTMLInputElement): void {
    const file = input.files?.[0];
    input.value = '';
    if (file) this.chose.emit(file);
  }
}
