/**
 * The families a model id can name.
 *
 * A closed set on purpose: it is what makes an unrecognised id *recognisably*
 * unrecognised, so it can be shown whole rather than mangled into a confident
 * wrong answer.
 */
const FAMILIES = ['opus', 'sonnet', 'haiku', 'fable'];

/**
 * What to call a model, given the id the CLI reports.
 *
 * `claude-opus-5[1m]` is what the wire says and *Opus 5* is what anyone calls
 * it. The pieces this has to survive, all of them taken from ids actually in the
 * transcript corpus rather than from a guess at the format:
 *
 * - a `claude-` prefix, or none at all — `opus`, `sonnet` and `haiku` appear bare
 * - a version written with hyphens for dots: `claude-opus-4-8` is 4.8
 * - a build date on the end: `claude-haiku-4-5-20251001`
 * - a bracketed variant: `[1m]`, the million-token window
 *
 * ⚠ **An unrecognised id is returned untouched**, never half-parsed. New models
 * arrive between releases, and a header confidently naming one it has never seen
 * is worse than one showing the id — the id is at least true, and it is the
 * thing worth reporting when the name looks wrong.
 *
 * The variant is dropped rather than shown: the only one in use marks the
 * million-token window, and the header states the window next to it in tokens
 * already.
 */
export function modelName(id: string | undefined): string | undefined {
  if (!id) return undefined;
  const parts = id
    .replace(/\[[^\]]*\]$/, '')
    .replace(/^claude-/, '')
    .split('-');
  const family = parts.shift();
  if (!family || !FAMILIES.includes(family)) return id;
  // A build date is not part of the name anybody says out loud.
  if (/^\d{8}$/.test(parts.at(-1) ?? '')) parts.pop();
  const named = family[0].toUpperCase() + family.slice(1);
  return parts.length ? `${named} ${parts.join('.')}` : named;
}
