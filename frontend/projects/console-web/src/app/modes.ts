/**
 * What a session may do without asking, in the CLI's own words.
 *
 * ⚠ **The stored name is not the shown name.** `default` displays as *Manual*,
 * which is why the modes feel like four with one called auto while the wire
 * carries six. Prettifying these here — title-casing `acceptEdits`, calling
 * `default` "Default" — would invent a vocabulary that disagrees with the CLI
 * the person is also using.
 *
 * Read off the 2.1.220 binary's own label table, not from memory. `rank` is its
 * ordering too: how much each mode lets through, `plan` lowest.
 */
export const MODES: Record<string, { title: string; rank: number }> = {
  plan: { title: 'Plan', rank: 0 },
  default: { title: 'Manual', rank: 1 },
  dontAsk: { title: "Don't Ask", rank: 1 },
  acceptEdits: { title: 'Accept edits', rank: 2 },
  auto: { title: 'Auto', rank: 3 },
  bypassPermissions: { title: 'Bypass Permissions', rank: 4 },
};

/**
 * How a mode should read on screen.
 *
 * An unknown mode is shown as it arrived rather than hidden or renamed: the CLI
 * gains modes between releases, and a console that silently drops one it has not
 * heard of would show a session as unrestricted-looking when it is anything but.
 */
export function modeTitle(mode: string | undefined): string | undefined {
  if (!mode) return undefined;
  return MODES[mode]?.title ?? mode;
}

/**
 * Whether a mode is one worth flagging rather than merely stating.
 *
 * The two the CLI itself colours as errors: nothing is asked before it happens.
 * Not a judgement of its own — it is the CLI's, kept because a phone four inches
 * wide cannot show everything and this is the one that changes what a glance at
 * the screen means.
 */
export function modeIsLoud(mode: string | undefined): boolean {
  return mode === 'bypassPermissions' || mode === 'dontAsk';
}
