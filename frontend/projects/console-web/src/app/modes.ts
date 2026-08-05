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
 *
 * The icons are ours — the CLI's are terminal glyphs and two of them are the
 * same ⏵⏵ for three different modes, which is fine beside a word and useless
 * standing alone. These have to be legible at 18px with no label, so each says
 * what the mode *does*: a raised hand asks, an open padlock does not.
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
 * The two the CLI itself colours as errors, and they are errors for opposite
 * reasons — in its own words, `bypassPermissions` "will not ask for your
 * approval before running potentially dangerous commands", where `dontAsk`
 * *skips* what would need approval "rather than prompting". One does the
 * dangerous thing unasked; the other quietly does not do the thing at all. The
 * rank above already says so: `dontAsk` sits with Manual at the bottom of the
 * dial, not with Auto at the top.
 *
 * Not a judgement of its own — it is the CLI's, kept because a phone four inches
 * wide cannot show everything and this is the one that changes what a glance at
 * the screen means.
 */
export function modeIsLoud(mode: string | undefined): boolean {
  return mode === 'bypassPermissions' || mode === 'dontAsk';
}

/**
 * The icon standing for a mode where there is no room for its name.
 *
 * ⚠ **A mode with no icon must not become an invisible one.** An unrecognised
 * mode gets a question mark rather than nothing, because the header's job here
 * is to say what the session may do — and a blank where that answer goes reads
 * as the careful setting, which is the one case it might not be.
 */
export function modeIcon(mode: string | undefined): string | undefined {
  if (!mode) return undefined;
  return MODES[mode]?.icon ?? 'help';
}

/**
 * The modes to offer, least allowed first.
 *
 * Sorted by the CLI's own rank so the menu reads as a dial rather than a set —
 * moving down the list is always moving toward asking less. `dontAsk` shares a
 * rank with `default` and is broken by name, so the order is stable rather than
 * whatever the object literal happened to be in.
 */
export function offeredModes(): { mode: string; title: string; icon: string; loud: boolean }[] {
  return Object.entries(MODES)
    .sort(([leftName, left], [rightName, right]) =>
      left.rank === right.rank ? leftName.localeCompare(rightName) : left.rank - right.rank,
    )
    .map(([mode, { title, icon }]) => ({ mode, title, icon, loud: modeIsLoud(mode) }));
}
