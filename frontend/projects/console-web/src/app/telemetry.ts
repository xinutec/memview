import { DOCUMENT, Injectable, inject } from '@angular/core';
import { NavigationEnd, Router } from '@angular/router';
import { TelemetryCore } from '@xinutec/ui-harness/telemetry';
import { filter } from 'rxjs';

/**
 * The Angular binding for the fleet's activity trace; the queue and transport
 * are shared in `@xinutec/ui-harness/telemetry`, which is built by plain tsc and
 * cannot carry an `@Injectable`. The console's API log says a session was told
 * something; only this says whether the person found the button or gave up.
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

    // Anything that threw. Without this a page that broke on its own left no mark,
    // and the failure looked like somebody losing interest.
    const view = this.doc.defaultView;
    view?.addEventListener(
      'error',
      (ev) => {
        // Two events share this name: a resource that failed to load has an element
        // target and no message; a script that threw has a message and no useful target.
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
   * A request that did not come back, or came back refused — the one thing in
   * the trace nobody did.
   */
  failure(url: string, status: number): void {
    this.core.record('fail', url, String(status));
  }

  /**
   * Something the app tried and could not do, where the person is not told: the
   * menu shows a value before the runner has agreed, and a refusal puts it back
   * silently.
   */
  note(what: string, detail: string): void {
    this.core.record('refused', what, detail);
  }

  /**
   * A number the page measured about itself, for the faults that only happen on
   * the device — a layout that settles wrongly on a phone and correctly in the
   * harness, where the timing IS the bug.
   */
  measured(what: string, detail: string): void {
    this.core.record('measured', what, detail);
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
