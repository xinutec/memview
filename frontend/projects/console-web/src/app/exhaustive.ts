/**
 * The case nothing is left for: a `switch` over a union compiles with a variant
 * missing, so a kind added in Rust would silently vanish from the transcript.
 * Calling this in the default makes that a build error.
 *
 * It does not throw: a phone often holds an older bundle than the runner, and
 * a crash on an unfamiliar event would make every new variant an outage.
 */
export function unhandled(variant: never): void {
  // Read only so the parameter counts as used; `never` has no members.
  void variant;
}
