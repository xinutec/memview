# memview

Read-only web viewer for the Claude memory markdown corpus (the per-project
`memory/` directory: YAML frontmatter + `[[wikilink]]` cross-references +
a curated `MEMORY.md` index) — and, increasingly, for what the sessions that
wrote that corpus were *doing*. See [The transcript side](#the-transcript-side).

- **Backend:** Rust (axum). Parses markdown with comrak; wikilinks and
  `(file.md)` index links are rewritten to the SPA route `/m/<name>`. The
  corpus is re-read from disk on every request, so a live Claude session's
  writes appear immediately.
- **Frontend:** Angular 22 + Material (zoneless), `frontend/`. Self-contained
  fonts (no third-party fetches) — it must render over the VPN.
- **Auth:** Nextcloud OAuth2 identity (copied from the `messages` app) with a
  stateless HMAC session cookie, plus a health-style **public share token**
  (`/share/<token>`, read-only, rotate = old link dies). Both are **inert
  unless configured**: without `SESSION_SECRET` the app serves open — that's
  the local dev mode.

## Views

| route | what |
| --- | --- |
| `/` | MEMORY.md, the curated index |
| `/m/<name>` | one memory, with backlinks, outlinks and dangling links |
| `/all` | every memory, grouped by type |
| `/search` | substring search with snippets |
| `/graph` | the corpus as a 3D link graph |
| `/agents` | which named session works where — owner only |
| `/sharing` | owner-only share-link management |

`/graph` draws every memory as a node and every resolvable `[[wikilink]]` as an
edge (`GET /api/graph`), laid out by a force simulation in `graph-layout.ts`.
Drag to turn, scroll to zoom, tap a memory to walk its neighbourhood 1–5 hops;
the section legend doubles as a filter.

Three choices worth knowing before changing it:

- **Canvas 2D with a hand-rolled projection, not WebGL.** A few hundred nodes
  cost nothing to draw, text labels stay trivial, and the bundle gains no
  dependency — the page has to render over the VPN with no third-party fetch.
- **Sections are positional, not just coloured.** Each MEMORY.md `## section`
  gets an anchor on a sphere and its members are pulled toward it, because with
  link forces alone — and ~half the corpus's links crossing sections — the
  curated colours smear uniformly through one ball. Measured on the live corpus,
  same-section pairs settle ~30% closer than cross-section ones.
- **Canvas gets nothing from the stylesheet.** It does not inherit the Material
  theme and does not repaint when the OS flips to dark, and a `light-dark()`
  token assigned to `fillStyle` is ignored *silently*. So data colours are fixed
  `hsl()`, only the chrome resolves tokens (through a probe element), a
  `prefers-color-scheme` listener repaints, and `expectCanvasLegible` reads the
  actual pixels in both schemes. dev-lint's `DL-CANVAS-SYSTEM-TOKEN` guards the
  static half.

## The transcript side

The corpus is what the sessions *wrote down*. Beside it, memview mines what they
actually **did**, from the session transcripts under `~/.claude/projects`. The
goal is to understand almost every part of it at a level above the shell: not to
reconstruct a command, but to be able to say in hindsight what it was doing.

Each stage's authoritative explanation is its module doc-comment; the chain is:

| module | question it answers |
| --- | --- |
| `shell.rs` + `shell.pest` | what commands does this script run? |
| `shell_ops.rs` | what does one command do — to which paths? |
| `python.rs` + `python.pest` | same, for the Python that Claude runs inline |
| `shell_files.rs` | resolved against a working directory, which files? |
| `activity.rs` | what *kind of work* was that — test, build, edit, deploy? |
| `commits.rs` | what did the repositories record, renames followed |
| `agents.rs` | who works where — the roster behind `/agents` |
| `doing.rs` | the timeline: agent · minute · repo · kind · count · verdict |
| `couse.rs` | which memories get used together in one turn |

**Derived, never verbatim.** No command line, no prompt, no output text reaches
any artefact — only typed structure and counts. The rule and its one lifted half
(timelines are allowed, as of 2026-08-02) are recorded in `doing.rs`.

Mining is offline and writes JSON beside the transcripts, which `scripts/sync.sh`
pushes to the pod:

```sh
cargo run --release --bin agents        # → ~/.claude/agents.json + doing.json
cargo run --release --bin couse         # → ~/.claude/couse.json
cargo run --release --bin bash-corpus > /tmp/bash-corpus.jsonl   # for the reports
cargo run --release --bin shell-report     -- /tmp/bash-corpus.jsonl  # the grammar
cargo run --release --bin shell-files      -- /tmp/bash-corpus.jsonl  # the semantics
cargo run --release --bin activity-report  -- /tmp/bash-corpus.jsonl [--sample KIND]
cargo run --release --bin python-report    -- /tmp/bash-corpus.jsonl [--why|--sample]
```

The `*-report` bins are how coverage is measured — what fraction of the real
corpus each layer can name, and what the biggest unnamed thing still is. Run
them rather than trusting a number written down here.

**Not built yet**, in the order to do it:

1. **The timeline is Bash-only.** Rows are pushed inside the `Bash` branch of
   `agents::scan_transcript`, so `Read`, `Write`, `Edit`, `Grep` and `Task` calls
   produce no activity — it currently means "what the sessions did *in the
   shell*". The tool calls are parsed a few lines away in the same function.
2. **The timeline has no page.** `/api/doing` is reachable only by curl.
3. **Episodes** — grouping rows into stretches of one intent. Deliberately
   deferred until there is a real activity stream to find the boundaries in; the
   rows already carry session, minute, agent, repository and verdict, so it needs
   no re-mining.

## Run (dev, Mac)

```sh
cd frontend && npm install && npm run build   # once, and after UI changes
MEMORY_DIR=~/.claude/projects/-Users-pippijn-Code/memory \
  STATIC_DIR=frontend/dist/memview-web/browser \
  nix develop -c cargo run
# → http://192.168.1.81:8091 (binds 0.0.0.0:8091 by default)
```

`ng serve` (in `frontend/`) proxies `/api` to `127.0.0.1:8091` for UI work.

## Environment

| var | default | meaning |
| --- | --- | --- |
| `MEMORY_DIR` | (required) | corpus directory |
| `BIND_ADDR` | `0.0.0.0:8091` | listen address |
| `STATIC_DIR` | unset | built SPA to serve; unset = API-only |
| `AGENTS_FILE` | unset | mined roster; unset = `/agents` serves nothing |
| `DOING_FILE` | unset | mined timeline; the one artefact cached, on mtime |
| `COUSE_FILE` | unset | mined co-use counts |
| `SHARE_STATE` | `share-state.json` | share-token persistence file |
| `PUBLIC_BASE_URL` | unset | base for composed share links |
| `SESSION_SECRET` | unset | enables auth; HMAC key for cookies |
| `NC_BASE_URL` / `NC_CLIENT_ID` / `NC_CLIENT_SECRET` / `NC_REDIRECT_URI` | — | NC OAuth2 client (required once auth is enabled) |
| `NC_INTERNAL_URL` | unset | server-side NC base (cluster Service DNS; sends `Host:` of `NC_BASE_URL`) — the isis hairpin fix |
| `ALLOWED_USERS` | — | comma-separated NC user allow-list; fail-closed |

## Deployment

`https://memview.xinutec.org` — isis, over the WireGuard VPN, behind the
Nextcloud sign-in wall. The manifests are generated from the Dhall model in the
infrastructure repo (`kubes/dhall/apps/memview.dhall` → `kubes/memview/k8s/`);
that repo's `memview/README.md` is the authority on the deployment, including
why the corpus is a volume and never part of the image.

Nothing about the corpus lives here: `scripts/sync.sh` pushes it — and the mined
artefacts — up from the Mac, one way, because the Mac is the root of truth and
isis is the exposed mirror.

## Verify

```sh
scripts/verify.sh   # cargo fmt+clippy+test, eslint, vitest, ng build, memory-lint
```

`memory-lint` is the corpus's own static analysis (`lint.rs`, run by the memory
repo's pre-commit gate as well): frontmatter shape, dangling and untyped links,
paths that no longer exist, memories that are used together but never linked —
and reachability, which is the rule worth knowing: every memory must be walkable
from `MEMORY.md` through links, however many hops away, but need not be indexed
in it.
