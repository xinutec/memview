/**
 * The families a model id can name — a closed set, so an unrecognised id is
 * recognisably unrecognised.
 */
const FAMILIES = ['opus', 'sonnet', 'haiku', 'fable'];

/**
 * What to call a model, given the id the CLI reports: `claude-opus-5[1m]` is
 * *Opus 5*. The shapes, all from ids in the corpus: a `claude-` prefix or none;
 * hyphens for dots (`claude-opus-4-8`); a build date on the end; a bracketed
 * variant. An unrecognised id is returned untouched — the id is at least true.
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
