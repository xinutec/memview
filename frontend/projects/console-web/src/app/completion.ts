import type { Commands, SlashCommand } from './models';

/** One row under the message box. */
export interface Suggestion {
  /** What the box holds once it is picked; absent for a row that only says what to type. */
  readonly fill?: string;
  readonly label: string;
  readonly detail: string;
}

/**
 * What to offer for what is typed, from what the session said it can be told:
 * its slash commands while the first word is typed, then that command's values.
 * The values come from the session — `/model` its models, `/output-style` its
 * styles — or from a hint listing them (`<low|medium|high>`); a hint that lists
 * none (`[name]`) is shown as what to type. Anything that is not a slash command
 * gets nothing.
 */
export function suggest(text: string, from: Commands | undefined): Suggestion[] {
  // Nothing is left to complete in a row that is exactly what is typed.
  return offered(text, from).filter((row) => row.fill !== text);
}

function offered(text: string, from: Commands | undefined): Suggestion[] {
  if (!from || !text.startsWith('/')) return [];
  const word = /^\/(\S*)$/.exec(text);
  if (word) return commandsFor((word[1] ?? '').toLowerCase(), from.commands);
  const arg = /^\/(\S+) (\S*)$/.exec(text);
  if (!arg) return [];
  const [, typed = '', partial = ''] = arg;
  const command = byName(typed.toLowerCase(), from.commands);
  if (!command) return [];
  const values = valuesOf(command, from);
  if (values.length === 0) {
    return command.argument_hint
      ? [{ label: command.argument_hint, detail: command.description }]
      : [];
  }
  const wanted = partial.toLowerCase();
  return values
    .filter((v) => v.label.toLowerCase().startsWith(wanted))
    .map((v) => ({ fill: `/${typed} ${v.label}`, label: v.label, detail: v.detail }));
}

/** The commands a typed prefix names, by name or alias; a name match first. */
function commandsFor(prefix: string, commands: readonly SlashCommand[]): Suggestion[] {
  const named = commands.filter((c) => c.name.toLowerCase().startsWith(prefix));
  const aliased = commands.filter(
    (c) => !named.includes(c) && (c.aliases ?? []).some((a) => a.toLowerCase().startsWith(prefix)),
  );
  return [...named, ...aliased].map((c) => ({
    // A command that takes nothing is complete as picked; one that takes something
    // leaves the cursor where its value goes.
    fill: c.argument_hint ? `/${c.name} ` : `/${c.name}`,
    label: `/${c.name}`,
    detail: c.argument_hint ? `${c.argument_hint} — ${c.description}` : c.description,
  }));
}

function byName(name: string, commands: readonly SlashCommand[]): SlashCommand | undefined {
  return commands.find((c) => c.name === name || (c.aliases ?? []).includes(name));
}

/** What a command takes, when that is a closed set. */
function valuesOf(command: SlashCommand, from: Commands): { label: string; detail: string }[] {
  if (command.name === 'model') {
    return from.models.map((m) => ({
      label: m.value,
      detail: `${m.display_name} · ${m.description}`,
    }));
  }
  if (command.name === 'output-style') {
    return from.output_styles.map((s) => ({ label: s, detail: '' }));
  }
  return choicesIn(command.argument_hint ?? '').map((c) => ({ label: c, detail: '' }));
}

/**
 * The choices a hint lists: `<low|high>` and `[on|off]` give theirs, a nested
 * argument keeps only its keyword (`disable [<server>|all]` gives `disable`), and
 * a placeholder (`<tokens>`) is no choice. A hint that lists nothing gives none.
 */
export function choicesIn(hint: string): string[] {
  const whole = /^[<[](.*)[>\]]$/.exec(hint.trim());
  if (!whole) return [];
  const inner = whole[1] ?? '';
  const parts: string[] = [];
  let depth = 0;
  let current = '';
  for (const ch of inner) {
    if (ch === '[' || ch === '<') depth++;
    if (ch === ']' || ch === '>') depth--;
    if (depth < 0) return [];
    if (ch === '|' && depth === 0) {
      parts.push(current);
      current = '';
    } else current += ch;
  }
  parts.push(current);
  if (parts.length < 2) return [];
  return parts
    .map((p) => p.trim().split(/\s+/)[0] ?? '')
    .filter((p) => p !== '' && !/^[<[(]/.test(p));
}
