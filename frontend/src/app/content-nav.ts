import { Directive, HostListener, inject } from '@angular/core';
import { Router } from '@angular/router';

/**
 * Intercepts clicks on links inside rendered-markdown innerHTML (which the
 * Angular router doesn't know about) and turns same-origin absolute paths
 * (/m/<name>) into router navigations instead of full page loads. External
 * links pass through untouched.
 */
@Directive({ selector: '[appContentNav]' })
export class ContentNav {
  private router = inject(Router);

  @HostListener('click', ['$event'])
  onClick(event: MouseEvent): void {
    if (event.ctrlKey || event.metaKey || event.shiftKey || event.button !== 0) return;
    // `instanceof` rather than `as HTMLElement`: a click can land on a text node
    // or an SVG element, neither of which has `closest`, and an assertion would
    // turn that into a TypeError at the top of every click handler.
    const target = event.target;
    if (!(target instanceof HTMLElement)) return;
    const anchor = target.closest('a');
    if (!anchor) return;
    const href = anchor.getAttribute('href');
    if (!href?.startsWith('/')) return;
    event.preventDefault();
    void this.router.navigateByUrl(href);
  }
}
