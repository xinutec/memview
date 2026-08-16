# The viewer

Read-only front end for the Claude memory corpus: markdown with YAML
frontmatter, `[[wikilink]]` references and a curated `MEMORY.md` index.

**Backend** Rust (axum), comrak for markdown; wikilinks and `(file.md)` index
links rewritten to `/m/<name>`. The corpus is re-read on every request — a live
session writes memories, and staleness beats the read cost.

**Frontend** Angular 22 + Material, zoneless, `frontend/`. Fonts are
self-contained: it must render over the VPN with no third-party fetch.

⚠ **One markdown parser.** `wikilinks_of` and `index_links` are public so graph,
backlinks, outlinks and index all walk comrak's AST. Hand-rolled `find("[[")`
scanners disagreed with comrak and each other — `[[-n "$target"]]` in a shell
snippet was reported as a link. Do not add a fourth.

## Routes

| route | what |
| --- | --- |
| `/` | MEMORY.md, the curated index |
| `/m/<name>` | one memory, with backlinks, outlinks, dangling links |
| `/all` | every memory, grouped by type |
| `/search` | substring search with snippets |
| `/graph` | the corpus as a 3D link graph |
| `/agents` | which named session works where — owner only |
| `/sharing` | owner-only share-link management |

Search hits render inline-only, so a snippet cut from a list yields words and no
stray `<li>`, and links are unwrapped to their text — one hit, one destination,
and a second link in a preview is an ambiguous phone tap target. Parsed by
comrak, never pattern-matched: `project_kubes_dhall_model` must not become
emphasis at its underscores.

## The graph

Every memory a node, every resolvable `[[wikilink]]` an edge (`GET /api/graph`),
laid out by a force simulation in `graph-layout.ts`. Drag to turn, scroll to
zoom, tap to walk a neighbourhood 1–5 hops; the section legend filters.

- **Canvas 2D with a hand-rolled projection, not WebGL.** A few hundred nodes are
  free to draw, text labels stay trivial, and the bundle gains no dependency.
- **Sections are positional, not only coloured.** Each `## section` gets a sphere
  anchor pulling its members in. With link forces alone — about half the corpus's
  links cross sections — the colours smear through one ball.
- **Canvas gets nothing from the stylesheet.** It does not inherit the Material
  theme, does not repaint on an OS dark-mode flip, and a `light-dark()` token
  assigned to `fillStyle` is ignored *silently*. So data colours are fixed
  `hsl()`, only chrome resolves tokens (via a probe element), a
  `prefers-color-scheme` listener repaints, and `expectCanvasLegible` reads
  pixels in both schemes. dev-lint's `DL-CANVAS-SYSTEM-TOKEN` guards the static
  half.

## Auth

Nextcloud OAuth2 identity, stateless HMAC session cookie (no database in this
app), plus a share token — `/share/<token>`, read-only, rotate kills the old
link.

**Inert unless configured**: without `SESSION_SECRET` it serves open, which is
dev mode. Configured, `ALLOWED_USERS` is the real gate and fails closed — any
Nextcloud account can complete the OAuth flow. The three auth keys are required
secretKeyRefs: a pod refusing to start beats one starting unguarded.

## Environment

| var | default | meaning |
| --- | --- | --- |
| `MEMORY_DIR` | (required) | corpus directory |
| `BIND_ADDR` | `0.0.0.0:8091` | listen address |
| `STATIC_DIR` | unset | built SPA to serve; unset = API-only |
| `AGENTS_FILE` | unset | mined roster; unset = `/agents` serves nothing |
| `DOING_FILE` | unset | mined timeline; the one cached artefact, on mtime |
| `COUSE_FILE` | unset | mined co-use counts |
| `SHARE_STATE` | `share-state.json` | share-token persistence |
| `PUBLIC_BASE_URL` | unset | base for composed share links |
| `SESSION_SECRET` | unset | enables auth; HMAC key for cookies |
| `NC_BASE_URL` / `NC_CLIENT_ID` / `NC_CLIENT_SECRET` / `NC_REDIRECT_URI` | — | NC OAuth2 client, required once auth is on |
| `NC_INTERNAL_URL` | unset | server-side NC base (cluster Service DNS, sends `Host:` of `NC_BASE_URL`) — the isis hairpin fix |
| `ALLOWED_USERS` | — | comma-separated NC allow-list; fail-closed |

## Deployment

`https://memview.xinutec.org` — isis, over WireGuard, behind Nextcloud sign-in.
Manifests are generated from `kubes/dhall/apps/memview.dhall` → `kubes/memview/k8s/`
in the infrastructure repo, whose `memview/README.md` is the authority, including
why the corpus is a volume and never in the image.

⚠ **DNS resolving to isis's WireGuard address is obscurity, not a firewall** —
the ingress answers on the public IP too. Sign-in plus allow-list is the gate.

⚠ **A push is not a deploy.** CI builds and pushes on any push to `main`, but
`:latest` is a fixed string nothing watches; `kubectl -n memview rollout restart
deploy/memview` is required. Rolling within seconds of a build picks up the
*previous* image, and Docker Hub's `tag_last_pushed` is not a usable signal. The
authoritative check is the artefact — compare prod's `main-*.js` hash against the
local build. Both pods are briefly Running during a rollout, so read the
replicasets, not `items[0]`.

The corpus does not live here: `scripts/sync.sh` pushes it and the mined
artefacts from the Mac, one way. Mac is root of truth, isis the exposed mirror.

## memory-lint

Static analysis of the corpus (`src/lint.rs`), run by this gate and by the memory
repository's own pre-commit hook: frontmatter shape, dangling and untyped links,
dead paths, memories used together but never linked.

**Severity is a ratchet.** A rule starts as a warning while violations are worked
down, and is promoted to error in a one-word edit at zero. Read the count off
`RULES`.

- **Reachability, not indexing** — every memory must be walkable from `MEMORY.md`
  through links, at any depth, but need not be listed in it.
- **`dangling-link` is never promoted.** The memory instructions call a link to
  an unwritten memory a marker for later, so it is a backlog counter.
- **The world check** (`dead-repo-path`, `unresolvable-code-root`) is the one
  family asking whether the graph is *true* rather than well-formed: it resolves
  every `~/Code/<repo>` against the real checkout root. The exemption is naming
  `~/Archive/<repo>`, so a true positive is fixed by saying where something went,
  never by silencing. Claims are gathered per top-level block — flattening a
  document let one archive banner clear every stale path below it.

⚠ `MemoryMeta.modified` is the file's **mtime**, not the frontmatter field of the
same name. A lint written against the wrong one can never fire.

## Development

```sh
cd frontend && npm install && npm run build   # once, and after UI changes
MEMORY_DIR=~/.claude/projects/-Users-pippijn-Code/memory \
  STATIC_DIR=frontend/dist/memview-web/browser \
  nix develop -c cargo run
# → http://192.168.1.81:8091
```

`scripts/dev.sh` does the same on `0.0.0.0:8091`; the Mac is headless, so use the
LAN address. `ng serve` in `frontend/` proxies `/api` to `127.0.0.1:8091`.

⚠ **`default-run = "memview"` in `Cargo.toml` is load-bearing.** A second
`src/bin/` target makes plain `cargo run` ambiguous and breaks `dev.sh` silently:
cargo refuses to choose, the dev server never starts, and a *stale* server
answers verification queries with old data.

Rust tests live in `tests/` against the public API, fixtures in
`tests/fixtures/memory/`; inline `#[cfg(test)]` is banned by dev-lint. Writing
them that way found two real bugs in the link graph.

### The gate

```sh
nix run ../dev-lint#gate -- . gate.json   # what the pre-commit hook runs
```

`gate.dhall` is the source; named checks (cargo fmt/clippy/test, eslint, e2e
typecheck, vitest, both builds, both layout harnesses, the graph-layout report,
`memory-lint`, shared dev-lint rules), each reported by name rather than as one
`&&` chain. `gate.json` is rendered from it and committed so the gate needs no
`dhall`; one check re-renders and diffs the two.

⚠ Slow enough to need a backgrounded run, not a foreground default timeout.
