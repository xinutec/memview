// Keep the last few published bundles in dist/console-live and remove the rest.
//
// `console-live` is what the phone loads, and it is filled by an rsync WITHOUT
// --delete on purpose: `ng build` empties its own output path, so a page loading
// mid-build would ask for a font and be handed HTML. Never deleting is what makes
// an upgrade invisible to whoever is looking at the console while it happens —
// and it is also why the directory only ever grew, to 273 files and 140 MB by
// 2026-08-11, of which one main-*.js was current.
//
// ⚠ **It cannot simply keep the newest set.** A phone with the app open is
// running a bundle the next publish supersedes, and it goes on asking that
// bundle for its lazy chunks and fonts until it reloads. Deleting on publish is
// how a session in someone's hand breaks halfway through a tap.
//
// So a generation is recorded, not inferred. Each publish writes the exact file
// list of the build it published; the keep set is the union of the last
// GENERATIONS lists. Content hashes mean an unchanged chunk appears in every
// list and is never a candidate, which is the behaviour wanted and would have
// been hard to get from mtimes.
//
// **The manifests live outside the served directory** (`dist/console-generations`),
// for the same reason the build output does: nothing that is not the app should
// be reachable from the app's own path.
//
// ⚠ **The first run deletes nothing.** Everything already in console-live is
// recorded as one legacy generation, so the 140 MB that predates this script
// falls off after GENERATIONS more publishes rather than being cut out from
// under a page that might be open right now. It is slower and it cannot break
// anything.
import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

/**
 * How many published generations stay reachable. A guess with a reason, not a
 * measurement: what it should be is how long a phone holds a page open without
 * reloading, and nothing here records that yet. Ten is enough that a page from
 * last week still finds its chunks, and small enough that the directory stays a
 * few tens of megabytes.
 */
export const GENERATIONS = 10;

/** Every file under `root`, as paths relative to it. */
function files(root) {
  const out = [];
  const walk = (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) walk(path);
      else out.push(relative(root, path));
    }
  };
  if (existsSync(root)) walk(root);
  return out.sort();
}

/**
 * Record what was just published and delete what no kept generation names.
 *
 * `now` is passed in rather than read here so a test can publish several
 * generations without sleeping between them; manifests sort by name, so the
 * stamp is what orders them.
 *
 * Returns what it did, for the caller to print — a function that both decides
 * and reports is one that cannot be tested without reading stdout.
 */
export function prune({ live, built, gens, generations = GENERATIONS, now = new Date() }) {
  if (!existsSync(live)) return { published: 0, seeded: null, kept: 0, removed: [], bytes: 0 };

  const published = files(built);
  // An empty build means `ng build` failed or wrote somewhere else. Pruning
  // against it would name nothing reachable and delete the app.
  if (published.length === 0) throw new Error(`${built} is empty — refusing to prune`);

  mkdirSync(gens, { recursive: true });
  const manifests = () =>
    readdirSync(gens)
      .filter((n) => n.endsWith('.txt'))
      .sort();

  // Seed, once: whatever is already live becomes the oldest generation, so this
  // run removes nothing. Written BEFORE the new manifest so it sorts first.
  let seeded = null;
  if (manifests().length === 0) {
    seeded = files(live).filter((f) => !published.includes(f));
    writeFileSync(join(gens, '00000000T000000Z-legacy.txt'), seeded.join('\n') + '\n');
  }

  const stamp = now.toISOString().replace(/[-:]/g, '').replace(/\.\d+Z$/, 'Z');
  writeFileSync(join(gens, `${stamp}.txt`), published.join('\n') + '\n');

  const all = manifests();
  const keep = all.slice(-generations);
  for (const gone of all.slice(0, -generations)) rmSync(join(gens, gone));

  const reachable = new Set();
  for (const name of keep) {
    for (const line of readFileSync(join(gens, name), 'utf8').split('\n')) {
      if (line) reachable.add(line);
    }
  }

  const removed = [];
  let bytes = 0;
  for (const file of files(live)) {
    if (reachable.has(file)) continue;
    const path = join(live, file);
    bytes += statSync(path).size;
    rmSync(path);
    removed.push(file);
  }

  // Directories a removed file was the last member of. One pass is enough: the
  // tree is two deep (media/, and the browser root).
  for (const entry of readdirSync(live, { withFileTypes: true })) {
    const path = join(live, entry.name);
    if (entry.isDirectory() && readdirSync(path).length === 0) rmSync(path, { recursive: true });
  }

  return { published: published.length, seeded, kept: keep.length, removed, bytes };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const dist = join(dirname(fileURLToPath(import.meta.url)), '..', 'dist');
  const result = prune({
    live: join(dist, 'console-live'),
    built: join(dist, 'console-build', 'browser'),
    gens: join(dist, 'console-generations'),
  });
  if (result.seeded) {
    console.log(
      `prune-live: recorded ${result.seeded.length} pre-existing files as one legacy generation`,
    );
  }
  const mb = (result.bytes / 1e6).toFixed(1);
  console.log(
    `prune-live: ${result.kept} generation(s) kept, ${result.removed.length} file(s) removed (${mb} MB)`,
  );
}
