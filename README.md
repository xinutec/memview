# memview

Read-only web viewer for the Claude memory markdown corpus (the per-project
`memory/` directory: YAML frontmatter + `[[wikilink]]` cross-references +
a curated `MEMORY.md` index).

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
| `SHARE_STATE` | `share-state.json` | share-token persistence file |
| `PUBLIC_BASE_URL` | unset | base for composed share links |
| `SESSION_SECRET` | unset | enables auth; HMAC key for cookies |
| `NC_BASE_URL` / `NC_CLIENT_ID` / `NC_CLIENT_SECRET` / `NC_REDIRECT_URI` | — | NC OAuth2 client (required once auth is enabled) |
| `NC_INTERNAL_URL` | unset | server-side NC base (cluster Service DNS; sends `Host:` of `NC_BASE_URL`) — the isis hairpin fix |
| `ALLOWED_USERS` | — | comma-separated NC user allow-list; fail-closed |

## Deployment target (planned)

isis, reachable over the WireGuard VPN like recall (`10.100.0.2:PORT`), gated
by the Nextcloud sign-in wall, with the memory dir pushed up from the Mac
(the Mac's one-way VPN means it can't be pulled). Manifests will live in
`pippijn/code/kubes/memview/` when that lands.

## Verify

```sh
scripts/verify.sh   # cargo fmt+clippy+test, eslint, vitest, ng build
```
