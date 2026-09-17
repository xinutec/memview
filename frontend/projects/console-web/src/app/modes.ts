/**
 * What a session may do without asking, in the CLI's own words. The stored
 * name is not the shown name: `default` displays as *Manual*. Read off the
 * 2.1.220 binary's label table; `rank` is its ordering, `plan` lowest.
 *
 * The icons are ours — the CLI's are terminal glyphs, two of them the same ⏵⏵
 * — and each says what the mode DOES: a raised hand asks, an open padlock does not.
 */
export const MODES: Record<string, { title: string; rank: number; icon: string }> = {
  plan: { title: 'Plan', rank: 0, icon: 'map' },
  default: { title: 'Manual', rank: 1, icon: 'pan_tool' },
  dontAsk: { title: "Don't Ask", rank: 1, icon: 'notifications_off' },
  acceptEdits: { title: 'Accept edits', rank: 2, icon: 'edit' },
  auto: { title: 'Auto', rank: 3, icon: 'auto_mode' },
  bypassPermissions: { title: 'Bypass Permissions', rank: 4, icon: 'lock_open' },
};

/**
 * How a mode should read on screen. An unknown mode is shown as it arrived:
 * the CLI gains modes between releases.
 */
export function modeTitle(mode: string | undefined): string | undefined {
  if (!mode) return undefined;
  return MODES[mode]?.title ?? mode;
}

/**
 * Whether a mode is worth flagging: the two the CLI itself colours as errors,
 * for opposite reasons — `bypassPermissions` does the dangerous thing unasked,
 * `dontAsk` quietly does not do it at all.
 */
export function modeIsLoud(mode: string | undefined): boolean {
  return mode === 'bypassPermissions' || mode === 'dontAsk';
}

/**
 * The icon standing for a mode where there is no room for its name. An
 * unrecognised mode gets a question mark, since a blank reads as the careful
 * setting.
 */
export function modeIcon(mode: string | undefined): string | undefined {
  if (!mode) return undefined;
  return MODES[mode]?.icon ?? 'help';
}

/**
 * The modes to offer, least allowed first, so the menu reads as a dial.
 * `dontAsk` shares a rank with `default` and is broken by name.
 */
export function offeredModes(): { mode: string; title: string; icon: string; loud: boolean }[] {
  return Object.entries(MODES)
    .sort(([leftName, left], [rightName, right]) =>
      left.rank === right.rank ? leftName.localeCompare(rightName) : left.rank - right.rank,
    )
    .map(([mode, { title, icon }]) => ({ mode, title, icon, loud: modeIsLoud(mode) }));
}
