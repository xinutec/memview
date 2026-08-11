// The pruner deletes files out of the directory the phone is served from, so it
// is tested against a real tree rather than reasoned about. Run by the gate:
//
//   node --test scripts/prune-live.test.mjs
//
// The cases are the two mistakes this script could make and one it must not be
// talked into: deleting a bundle a phone is still asking for, keeping every
// bundle for ever, and pruning against a build that is not there.
import { strict as assert } from 'node:assert';
import { existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import { prune } from './prune-live.mjs';

/** A publish: `ng build` writes the build directory, rsync copies it into live. */
function publish(root, names, { generations, minute }) {
  const built = join(root, 'console-build', 'browser');
  const live = join(root, 'console-live');
  rmSync(built, { recursive: true, force: true }); // ng build empties its output path
  mkdirSync(built, { recursive: true });
  for (const name of names) writeFileSync(join(built, name), name);
  for (const name of names) writeFileSync(join(live, name), name); // rsync, no --delete
  return prune({
    live,
    built,
    gens: join(root, 'console-generations'),
    generations,
    now: new Date(Date.UTC(2026, 7, 11, 12, minute)),
  });
}

function tree() {
  const root = mkdtempSync(join(tmpdir(), 'prune-live-'));
  mkdirSync(join(root, 'console-live'), { recursive: true });
  return root;
}

const live = (root) => readdirSync(join(root, 'console-live')).sort();

test('the first run deletes nothing it did not put there', () => {
  const root = tree();
  writeFileSync(join(root, 'console-live', 'main-OLD.js'), 'a bundle a phone may still hold');
  const result = publish(root, ['main-A.js', 'index.html'], { generations: 2, minute: 1 });
  assert.deepEqual(result.removed, []);
  assert.deepEqual(result.seeded, ['main-OLD.js']);
  assert.deepEqual(live(root), ['index.html', 'main-A.js', 'main-OLD.js']);
});

test('a bundle older than the window goes, and the window keeps the ones behind it', () => {
  const root = tree();
  for (const [minute, name] of [
    [1, 'main-A.js'],
    [2, 'main-B.js'],
    [3, 'main-C.js'],
  ]) {
    publish(root, [name, 'index.html'], { generations: 2, minute });
  }
  // Two generations kept: C is current, B is what an unreloaded phone holds. A
  // is three publishes back and nothing names it.
  assert.deepEqual(live(root), ['index.html', 'main-B.js', 'main-C.js']);
});

test('a chunk every build shares is never a candidate', () => {
  const root = tree();
  for (const [minute, name] of [
    [1, 'main-A.js'],
    [2, 'main-B.js'],
    [3, 'main-C.js'],
  ]) {
    publish(root, [name, 'chunk-SHARED.js'], { generations: 1, minute });
  }
  // Content-hashed, so an unchanged chunk keeps its name and is republished
  // every time — it is in the newest manifest and stays whatever the window is.
  assert.deepEqual(live(root), ['chunk-SHARED.js', 'main-C.js']);
});

test('a build that is not there prunes nothing', () => {
  const root = tree();
  publish(root, ['main-A.js'], { generations: 2, minute: 1 });
  rmSync(join(root, 'console-build', 'browser'), { recursive: true });
  mkdirSync(join(root, 'console-build', 'browser'), { recursive: true });
  // Nothing published names anything, so every live file would look unreachable.
  // Refusing is the difference between a failed build and an empty console.
  assert.throws(
    () =>
      prune({
        live: join(root, 'console-live'),
        built: join(root, 'console-build', 'browser'),
        gens: join(root, 'console-generations'),
        generations: 2,
      }),
    /refusing to prune/,
  );
  assert.deepEqual(live(root), ['main-A.js']);
});

test('a directory the last file left is removed with it', () => {
  const root = tree();
  mkdirSync(join(root, 'console-live', 'media'));
  writeFileSync(join(root, 'console-live', 'media', 'font-OLD.woff2'), 'x');
  publish(root, ['main-A.js'], { generations: 1, minute: 1 });
  publish(root, ['main-B.js'], { generations: 1, minute: 2 });
  assert.equal(existsSync(join(root, 'console-live', 'media')), false);
});
