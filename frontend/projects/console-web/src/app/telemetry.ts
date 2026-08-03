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

    // Anything that threw. Until this existed the trace showed only what the
    // person did and what the API refused, so a page that broke on its own left
    // no mark at all — the failure looked like somebody losing interest.
    const view = this.doc.defaultView;
    view?.addEventListener(
      'error',
      (ev) => {
        // Two different events share this name. A resource that failed to load
        // has an element as its target and no message; a script that threw has
        // a message and no useful target.
        const target = ev.target;
        const source =
          target instanceof HTMLElement
            ? (target.getAttribute('src') ?? target.getAttribute('href') ?? target.tagName)
            : undefined;
        if (source) this.core.record('broke', source, target?.constructor.name ?? null);
        else this.core.record('threw', ev.message || 'error', where(ev));
      },
      // Capture, because a resource error does not bubble.
      { capture: true },
    );
    view?.addEventListener('unhandledrejection', (ev) => {
      this.core.record('threw', reason(ev.reason), 'promise');
    });

    this.core.start();
  }

  /**
   * A request that did not come back, or came back refused.
   *
   * Its own kind rather than a tap, because it is the one thing in the trace
   * nobody did — and because reading a log for "what went wrong" should not mean
   * inferring it from a gap between two navigations.
   */
  failure(url: string, status: number): void {
    this.core.record('fail', url, String(status));
  }
}

/** Where a script error came from, when the browser says. */
function where(ev: ErrorEvent): string | null {
  return ev.filename ? `${ev.filename}:${ev.lineno}:${ev.colno}` : null;
}

/** A rejection's reason as one line, whatever it happens to be. */
function reason(value: unknown): string {
  if (value instanceof Error) return value.message;
  return typeof value === 'string' ? value : JSON.stringify(value ?? null);
}
