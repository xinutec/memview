import { DOCUMENT, Injectable, inject } from '@angular/core';
import { NavigationEnd, Router } from '@angular/router';
import { TelemetryCore } from '@xinutec/ui-harness/telemetry';
import { filter } from 'rxjs';

/**
 * The Angular binding for the fleet's activity trace.
 *
 * The queue, the flush policy and the transport are shared and tested once in
 * `@xinutec/ui-harness/telemetry`; what has to live in the app is the framework
 * binding, because an `@Injectable` cannot cross that boundary (the package is
 * built by plain tsc, and a decorated class arriving without Ivy definitions
 * fails a production build on `JIT compiler unavailable`).
 *
 * It matters more here than in the viewer. The console's API log says a session
 * was told something; it cannot say whether the person meant to send it, or
 * found the button, or gave up. When the phone is the client and the screen is
 * four inches wide, that is the only record of how it went.
 */
@Injectable({ providedIn: 'root' })
export class Telemetry {
  private readonly router = inject(Router);
  private readonly doc = inject(DOCUMENT);
  private readonly core = new TelemetryCore(this.doc, {});

  /** Wire the two capture points. Called once from the app shell; idempotent. */
  init(): void {
    if (this.core.started) return;

    this.router.events
      .pipe(filter((e): e is NavigationEnd => e instanceof NavigationEnd))
      .subscribe((e) => this.core.record('nav', e.urlAfterRedirects, null));

    // Capture phase, so a tap is seen even where a handler stops propagation.
    this.doc.addEventListener('click', (ev) => this.core.recordTap(ev.target, this.router.url), {
      capture: true,
    });

    this.core.start();
  }
}
