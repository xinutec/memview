/**
 * The case nothing is left for.
 *
 * A `switch` over a closed union with no default compiles happily when a variant
 * is missing — TypeScript only checks the cases you wrote, not the ones you did
 * not. So a kind added in Rust, regenerated into `Event`, and never folded here
 * would arrive, match nothing, and vanish: no error, no log, a line missing from
 * a transcript nobody knows is short.
 *
 * ⚠ **It does not throw, and that is the point.** A phone can be holding an
 * older bundle than the runner it is talking to — that is the normal state of
 * this app for the minutes between an upgrade and a reload — and a client that
 * crashed on an unfamiliar event would turn every new variant into an outage.
 * The check belongs at build time, where the person who added the variant is;
 * at run time the honest thing is to carry on and draw what is understood.
 *
 * So: passing anything but `never` is a compile error, and running it is a no-op.
 */
export function unhandled(variant: never): void {
  // Read once so the parameter is used, and never otherwise: `never` has no
  // members, so there is nothing here to do with it.
  void variant;
}
