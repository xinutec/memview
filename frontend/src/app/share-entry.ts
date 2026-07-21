import { Component, effect, inject, input } from '@angular/core';
import { Router } from '@angular/router';

import { setShareToken } from './share-token';

/**
 * Landing route for a share link (/share/<token>): stash the token so the
 * interceptor sends it with every API call, then continue to the index. The
 * transient URL is replaced so history/bookmarks don't keep the raw token
 * page (the recall wrapper's callback-persistence lesson).
 */
@Component({
  selector: 'app-share-entry',
  templateUrl: './share-entry.html',
})
export class ShareEntry {
  private router = inject(Router);

  readonly token = input.required<string>();

  constructor() {
    effect(() => {
      setShareToken(this.token());
      void this.router.navigateByUrl('/', { replaceUrl: true });
    });
  }
}
