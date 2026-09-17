// A REAL console runner, isolated, for the two-device sync test.
//
// ⚠ **Isolation is asserted, not configured.** A console started with an
// incomplete environment does not fail — it falls back to `$HOME/.claude`, lists
// the real conversations and spawns the real CLI over them. That happened while
// this was being written. So the environment is built from nothing rather than
// inherited, `CLAUDE_BIN` is the stub that cannot reach the network, and `start`
// refuses to return until the runner has shown it can see the fixture and
// nothing else.
import { spawn, spawnSync, type ChildProcess } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';

/** The conversations the fixture describes, and the only ones a correctly
 *  isolated runner can see.
 *
 *  One per test rather than one shared: a draft is keyed by conversation and the
 *  runner keeps it across tests, so two tests on one id race over whose text the
 *  second device pulls. */
export const CONVERSATIONS = [
  '11111111-1111-1111-1111-111111111111',
  '22222222-2222-2222-2222-222222222222',
  '33333333-3333-3333-3333-333333333333',
  '44444444-4444-4444-4444-444444444444',
];

/**
 * Where the fixture says it ran, and it has to satisfy two rules at once.
 *
 * It must EXIST, because `/api/past` filters on `Config::resolve`, which
 * canonicalises the recorded directory — an invented path is dropped. And it
 * must NOT be under /tmp, /private/tmp or $TMPDIR, because the runner treats a
 * conversation there as a disposable probe and does not list it either. So it is
 * a real directory inside the workspace's gitignored `target/`.
 */
function where(): string {
  const dir = join(repo(), 'target', 'sync-fixture', 'demo');
  mkdirSync(dir, { recursive: true });
  return dir;
}

/** The workspace root, found rather than counted in `../`s — this file is
 *  loaded by Playwright from a cwd it chooses. */
function repo(): string {
  let at = __dirname;
  while (!existsSync(join(at, 'flake.nix'))) {
    const up = dirname(at);
    if (up === at) throw new Error('no workspace root above ' + __dirname);
    at = up;
  }
  return at;
}

/** The console binary. Named, never searched for, and absent is an ERROR: a
 *  skipped test reads exactly like a passing one. */
function binary(): string {
  const path = process.env['CONSOLE_BIN'] ?? join(repo(), 'target/debug/console');
  if (spawnSync(path, ['--help'], { encoding: 'utf8' }).error) {
    throw new Error(
      `no console binary at ${path}. Build it with ` +
        '`cargo build -p console --bin console`, or set CONSOLE_BIN.',
    );
  }
  return path;
}

/** A scratch tree holding the fixture conversations and nothing else. */
function fixture(): string {
  const cwd = where();
  const root = mkdtempSync(join(tmpdir(), 'console-sync-'));
  const project = join(root, 'projects', '-home-example-Code-demo');
  mkdirSync(project, { recursive: true });
  mkdirSync(join(root, 'home'), { recursive: true });
  for (const id of CONVERSATIONS) {
    writeFileSync(
      join(project, `${id}.jsonl`),
      [
        JSON.stringify({ type: 'system', subtype: 'init', cwd, session_id: id }),
        JSON.stringify({
          type: 'user',
          cwd,
          message: { role: 'user', content: [{ type: 'text', text: 'hello' }] },
          timestamp: '2026-09-17T10:00:00.000Z',
        }),
      ].join('\n') + '\n',
    );
  }
  return root;
}

export interface Runner {
  base: string;
  stop: () => void;
}

/** Start a runner serving the built console on `port`, and resolve once it
 *  answers AND has proved it is looking at the fixture.
 *
 *  `staticDir` is relative to the WORKSPACE, not to the caller: Playwright picks
 *  its own cwd, and resolving against that served a directory outside the repo
 *  while the app answered with the runner's fallback — a page that renders, so
 *  the failure arrived as "no textarea" rather than as a missing bundle. */
export async function start(port: number, staticDir: string): Promise<Runner> {
  const served = resolve(repo(), staticDir);
  if (!existsSync(join(served, 'index.html'))) {
    throw new Error(`no built console at ${served}. Build it with \`pnpm run build:console\`.`);
  }
  const root = fixture();
  const child = spawn(binary(), [], {
    // ⚠ Nothing inherited. The parent's HOME and CLAUDE_PROJECTS_DIR are the
    // real ones, and inheriting either is what pointed a runner at the live
    // corpus.
    env: {
      PATH: '/usr/bin:/bin',
      HOME: join(root, 'home'),
      USER: process.env['USER'] ?? 'test',
      CONSOLE_HOME: join(root, 'home'),
      CLAUDE_PROJECTS_DIR: join(root, 'projects'),
      // The directory the FIXTURE says it ran in, not the scratch root: `/api/past`
      // is filtered to what this allows, and that filter is what makes the check
      // below able to tell one corpus from another.
      CONSOLE_DIRS: dirname(where()),
      // Cannot spawn a real session, and cannot cost anything.
      CLAUDE_BIN: join(repo(), 'console/tests/fixtures/stub-cli'),
      // With no TLS configured this IS the plaintext listener; `desk` is the
      // companion to a TLS bind, and the console refuses anything but loopback
      // without client authentication.
      BIND_ADDR: `127.0.0.1:${port}`,
      CONSOLE_DESK_ADDR: '127.0.0.1:0',
      // A port nothing listens on: the usage dashboard must never be asked.
      CONSOLE_USAGE_URL: 'http://127.0.0.1:1/none',
      STATIC_DIR: served,
    },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  // Kept so a runner that dies takes its reason with it into the error. The
  // console writes its own startup failures here and nowhere else.
  let log = '';
  child.stdout?.on('data', (chunk: Buffer) => (log += chunk.toString()));
  child.stderr?.on('data', (chunk: Buffer) => (log += chunk.toString()));

  const base = `http://127.0.0.1:${port}`;
  for (let attempt = 0; attempt < 150; attempt++) {
    if (child.exitCode !== null) throw new Error(`runner exited ${child.exitCode}:\n${log}`);
    const seen = await conversationsSeen(`${base}/api/past`);
    if (seen) {
      // ⚠ **The isolation check, and it is an EQUALITY.** A runner that fell
      // back to the real corpus answers every route perfectly well, with
      // somebody else's conversations in it — and one that can see nothing at
      // all would satisfy any check phrased as "no strangers". Both are refused
      // by insisting on exactly the fixture.
      const missing = CONVERSATIONS.filter((id) => !seen.includes(id));
      const strangers = seen.filter((id) => !CONVERSATIONS.includes(id));
      if (missing.length || strangers.length) {
        stop(child, root);
        throw new Error(
          'runner is NOT isolated: ' +
            `missing ${missing.join(', ') || 'none'}; ` +
            `unexpected ${strangers.join(', ') || 'none'}`,
        );
      }
      return { base, stop: () => stop(child, root) };
    }
    await new Promise((settle) => setTimeout(settle, 100));
  }
  stop(child, root);
  throw new Error(`runner never answered on ${base}:\n${log}`);
}

/**
 * The conversations the runner can see, or `undefined` while it is not
 * answering yet.
 *
 * ⚠ **`/api/past`, and NOT the `gists` on `/api/state`.** The first version of
 * this asked for gists — which are written by an ASYNCHRONOUS sweep, so at the
 * moment this polls they are almost always none, and "no stranger among them"
 * came back true for a runner pointed straight at the real corpus. Ablated by
 * doing exactly that: the suite passed. `/api/past` is read from the directory
 * on the way out, so it answers about the corpus that is actually configured.
 */
async function conversationsSeen(url: string): Promise<string[] | undefined> {
  const body: unknown = await fetch(url)
    .then((res) => (res.ok ? (res.json() as Promise<unknown>) : undefined))
    .catch(() => undefined);
  if (!Array.isArray(body)) return undefined;
  return body.flatMap((row: unknown) =>
    typeof row === 'object' && row !== null && 'id' in row && typeof row.id === 'string'
      ? [row.id]
      : [],
  );
}

function stop(child: ChildProcess, root: string): void {
  child.kill('SIGKILL');
  rmSync(root, { recursive: true, force: true });
}
