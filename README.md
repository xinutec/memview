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
actually **did**, from the session transcripts under `~/.claude/projects`.

**The aim is to understand what a command actually did, to a near-complete
degree, without ever running it.** Not a summary a level above the shell — the
execution itself: which constant a name was bound to, which three files a loop
over a literal list touched, which machine a path belonged to, which program a
heredoc carried and what *that* did in turn. A command is understood when we can
say what it named and what it changed; the exceptions should be few, and each
one should be a known kind rather than a shrug.

Read as a language problem, that means the reader is an **abstract
interpreter**: it evaluates as far as the text determines and stops, never
guessing past the end of what it knows. What it cannot determine is recorded as
undetermined and counted, so the gap is a number rather than a silence.

Each stage's authoritative explanation is its module doc-comment; the chain is:

| module | question it answers |
| --- | --- |
| `reader/src/shell.rs` + `shell.pest` | what commands does this script run? |
| `reader/src/shell_ops.rs` | what does one command do — to which paths? |
| `reader/src/python.rs` + `python.pest` | same, for the Python that Claude runs inline |
| `reader/src/shell_files.rs` | resolved against a working directory, which files? |
| `reader/src/activity.rs` | what *kind of work* was that — test, build, edit, deploy? |
| `reader/src/doing.rs` | the timeline: agent · minute · repo · kind · count · verdict |
| `src/commits.rs` | what did the repositories record, renames followed |
| `src/agents.rs` | who works where — the roster behind `/agents` |
| `src/couse.rs` | which memories get used together in one turn |

**The reader is its own crate, and the split is a boundary rather than tidying.**
Everything that answers *what does this text mean* lives in `reader/`; everything
that answers *whose work was it* stays in the viewer. That is what lets the agent
console — which spawns processes on the root-of-truth Mac — read a command
without linking a viewer that carries routes, auth and configuration. See
`reader/src/lib.rs` for why a leaf makes that safe, and the bug the two crates
already paid for by re-deriving the same knowledge separately.

**Derived, never verbatim.** No command line, no prompt, no output text reaches
any artefact — only typed structure and counts. The rule and its one lifted half
(timelines are allowed, as of 2026-08-02) are recorded in `doing.rs`.

### One command through the chain

The doc-comments explain each stage; what they cannot show is how a path
survives all four, or where it stops. A real shape from the corpus:

```
cd health && nix develop -c bash -c "sed -i 's/a/b/' src/geo/velocity.ts"

shell.rs        two commands. The inner script is ONE WORD here — the quoting is
                the parser's job, the meaning is not.
shell_ops.rs    ChangeDir{to:"health"} · Carries("-c") unwraps the argv to `bash
                -c …`, which is Nested{script} — parsed again, in its own scope
                → Transform{program:"s/a/b/", in_place:true, paths:[…]}
shell_files.rs  cwd is now …/health, so the relative path resolves. in_place
                makes it a write rather than a read.
activity.rs     an edit.
```

**Where it stops, and each stop is deliberate.** If `cd`'s target cannot be
resolved the working directory becomes *unknown*, not stale — so the relative
path names nothing rather than the wrong thing. If the outer command were `ssh`,
everything inside belongs to that machine and never reaches the local index. If
`sed`'s operand were `$F`, it is refused today (README Roadmap, item 1).

### Changing the reader

The reports are the method, not a status line. The loop:

1. **Rebuild the corpus first.** It moves fast — 87,918 calls on 1 August,
   134,004 on the 6th — and a coverage claim against a stale one is worthless.
2. Change the table or the grammar.
3. Re-run the report. The number that matters is the one that *moved*, and the
   failure list is the next thing to build.
4. **Ablate the test.** Undo the change; the new test must fail. Twice this month
   a test that could not fail was found this way and no other.

Two ways a figure has misled here, each written up where it happened: a rate that
hid a trade (`reader/examples/tree-sitter-probe.rs`) and a census that counted text
already read (`reader/src/bin/opacity.rs`).

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
cargo run --release --bin opacity          -- /tmp/bash-corpus.jsonl  # what nothing reads
```

The `*-report` bins are how coverage is measured — what fraction of the real
corpus each layer can name, and what the biggest unnamed thing still is. Run
them rather than trusting a number written down here. `opacity` is the fourth and
answers the opposite question: of the text these commands carry, how much does
nobody look inside, and who handed it over.

### Roadmap

**Where it reaches today** (2026-08-06, 134,004 Bash calls from 1,205
transcripts): 99.7% of 98,321 distinct commands parse; **98.9% of 746,241 simple
commands are understood**; 9,006 Python programs are read inside the shell that
ran them. Nested shells (`nix -c`, `bash -c`, `nix-shell --run`) are followed;
`ssh`/`kubectl`/`docker` are followed and filed against the machine, never here.

⚠ **That denominator counts commands *run*, not commands written** — 94,377 of
them exist because a determinate loop was run out. A percentage against it is
not comparable with one from before loops were unrolled, which is why the tool
prints the two apart.

**The distance left to the aim, and the reader now states it itself.** An
undetermined subject used to vanish: a word refused by the path guard left no
trace, so a command that used a file nobody can name was recorded exactly like a
command that used none. It is now counted and shown — `subjects not named` in
`shell-files`, with the words that stood for them.

Measured 2026-08-13 over 120,427 Bash calls: **3,007 uses, 714 distinct**, led by
`$f` (837), `$d` (166) and `$p` (129). Only refusals the text *could* have
determined are counted — a bare `src`, a pattern, a git refspec are refused for
good reasons and stay uncounted, or the figure would be noise nobody can act on.

⚠ **That is the shell's figure alone, and the Python side is worse.** Stated as
"1.7% of all file uses" here until 2026-08-13, which read as though it covered
everything the reader does. It does not: `shell_ops::paths` is where the counter
sits, and a Python program's subjects never pass through it.

| | named | not named |
| --- | --- | --- |
| shell | 173,055 uses | 3,007 |
| python | 11,876 of 16,065 operations (73.9%) | 4,189 computed, f-string or loop variable |
| python, refused at the shell boundary | 11,578 of 11,876 kept (97.5%) | 298 |
| **together** | **173,055** | **7,494 — 95.9% named** |

The Python reader does own up to its own — `Tally::unresolved` has recorded them
all along, and `python-report` prints them. What is missing is that they never
reach `Extract::unnamed`, so the shell-side report and this file both counted a
smaller denominator than the work they describe.

⚠ **Not all of that 1.7% is out of reach, and this section said otherwise until
2026-08-13.** It claimed the residue was loop variables over globs and `$(…)`,
"the two things that genuinely are not in the text". Measured rather than
asserted, the 6,563 `for` loops break down as 4,324 already run out and 2,239 not,
and the largest unrun class is neither:

| not run out | | what it can become |
| --- | --- | --- |
| `$(seq N M)`, constant bounds | 1,029 | **exact values** — the reader can evaluate it |
| a glob | 735 | a bounded subset of a known pattern, at best |
| some other `$(…)` | 365 | genuinely opaque, now and always |
| a variable | 104 | depends what bound it |

So the honest statement is that roughly half of what remains is *arithmetic the
reader declines to do*, not information the text lacks. `determinate` rejects any
word containing `$`, which folds `$(seq 1 18)` exactly as it was folded before
unrolling existed — the `seq` half of that limitation was never built, though
this file and memview#92 both read as though it had been.

Below that, the remaining unread commands are not a structural gap: the list is
headed by `dhall-to-json`, `k3s`, `screen`, `journalctl` — all missing rows in
the verb table rather than shapes the reader cannot express. Adding a row is
worth doing when the command names files; most of these do not.

Then the timeline, which is a separate thread: it is **Bash-only** (rows are
pushed inside the `Bash` branch of `agents::scan_transcript`, so `Read`, `Write`,
`Edit`, `Grep` and `Task` produce no activity), it has **no page** — `/api/doing`
is curl-only — and it has no **episodes**, the grouping of rows into stretches of
one intent.

**Deliberately not done**, so none of it is re-opened on instinct. Each was
decided from a measurement kept with the thing it decided:

| not done | why, in one line | where the numbers are |
| --- | --- | --- |
| a third-party parser | swapping loses more than it gains | `reader/examples/tree-sitter-probe.rs` |
| a third language reader | what is left is file content and commit messages | `reader/src/bin/opacity.rs` |
| parsing regexes | biggest by volume, but a regex names no file | `reader/src/bin/opacity.rs` |
| opening scripts on disk | what `deploy.sh` held *then* is not recoverable | — |
| reading `node -e` | a query tool, not an editor | `reader/src/shell_ops.rs` |

## The console

A second application in this repository: a front end for the *live* Claude Code
sessions on the Mac — start one, watch it work, send it instructions.

```sh
./scripts/console-upgrade.sh  # build, install, and move a RUNNING one onto it
./scripts/console.sh          # a one-off by hand, loopback only
```

⚠ **On the Mac it runs as a launchd service**, `org.xinutec.agent-console`
(declared in `xinutec-infra/mac-mini/hm-agents.nix`), so it comes up with the
machine and survives the terminal it was started from. **Restart it with
`console-upgrade.sh`, never `launchctl kickstart -k`**: kickstart sends SIGTERM,
which is the console's deliberate *stop* path and takes every open session with
it. The upgrade signal is SIGUSR2 — `Roster::handover` `execve`s the binary,
keeping the same pid, so the `claude` children never notice.

It is deliberately **not** part of the viewer. memview is read-only over
documents and runs on an internet-facing host; the console runs subprocesses on
the root-of-truth machine. They share a repository, a toolchain and a gate, and
nothing else: `console/` is its own crate that links nothing from `src/`, the
image builds `--bin memview` so the console binary cannot ride along into a
container, and the UI is its own Angular project.

**The gate.** Without `CONSOLE_TLS_*` set it refuses to listen anywhere but
loopback — the house LAN is not a trusted network. With them it requires a
client certificate whose *public key* is pinned, and serves nobody else — while
keeping a plaintext socket on `127.0.0.1:8096` for this machine, since the Mac is
headless and an SSH forward has no certificate to present. No CA
and no PKI: one console, a known set of devices, and a fingerprint that survives
the certificate being reissued because it is taken over the key. Adding a device
is a line; revoking one is deleting it. A refused key is logged with its
fingerprint, which is how you enrol the next one.

**The phone.** A Pixel with a key generated in its StrongBox — non-exportable, so
no Xinutec server ever holds a credential the console would accept. Three scripts,
in order, and then `console.sh` finds the material on its own:

```sh
./scripts/console-identity.sh                                    # the Mac's own key
nix develop ~/Code/recall#android --command console/android/deploy.sh
nix develop ~/Code/recall#android --command ./scripts/enrol.sh   # checks, then pins
```

`enrol.sh` will not pin a key on a claim it has not checked: a challenge it
generated seconds earlier, every signature in the chain, a Google root held in
this repository, Google's revocation list, StrongBox on both the record and the
key, an origin of GENERATED, and an authentication requirement the hardware
enforces. See [console/src/attest.rs](console/src/attest.rs) and
[console/android/README.md](console/android/README.md).

**Away from the house**, the Mac dials *out*: `console.sh` opens an SSH tunnel to
isis, which listens on its VPN address and hands the bytes back down it. The TLS
session terminates at the Mac's own gate, so isis carries ciphertext and holds no
key that opens anything — and the Mac binds loopback only, with no firewall
exception anywhere.

**Picking up where you left off.** The sessions page lists conversations already
on disk, by the name each gave itself, and resumes one in a process of the
console's own. It refuses any that something else appears to be using — a running
`claude` naming it, or a transcript written in the last two minutes — because two
processes on one transcript both append and neither sees the other's turns.

⚠ The console **cannot attach to a running session**, and neither can anything
else local: a `claude --remote-control` session talks to Anthropic over HTTPS with
no local endpoint at all. Resume is for conversations that have been closed. A
resumed one also starts with an empty view — `--resume` restores the CLI's
context, not the console's — which is the next thing to fix.

[docs/agent-console.md](docs/agent-console.md) is the authority on the design and
the threat model.

| var | default | meaning |
| --- | --- | --- |
| `CONSOLE_DIRS` | `~/Code` | colon-separated roots a session may start in |
| `CONSOLE_MODEL` | unset | model for spawned sessions; unset = the CLI's own |
| `CONSOLE_PERMISSION_MODE` | unset | see below |
| `CLAUDE_BIN` | `claude` | the CLI to spawn |
| `BIND_ADDR` | `127.0.0.1:8097` | must be loopback unless the gate is configured |
| `CONSOLE_DESK_ADDR` | `127.0.0.1:8096` | plaintext socket for this machine, only when the gate is on |
| `CONSOLE_TLS_CERT` / `CONSOLE_TLS_KEY` | unset | the console's own PEM certificate and key |
| `CONSOLE_CLIENT_KEYS` | unset | comma-separated SHA-256 pins of the client keys admitted |

**Approvals.** With `CONSOLE_PERMISSION_MODE=manual` the session asks before it
runs anything, the console shows the question — the tool, its arguments, the
CLI's own sentence — and nothing happens until someone answers; a refusal carries
a reason the session is told. Sessions blocked on a question say *waiting for
you* in the list.

⚠ Left on the CLI's **default** mode a headless session refuses every tool call
that needs permission and asks nobody, so it can converse and little else —
measured, not assumed. `manual` is the useful setting now that approvals exist;
`acceptEdits` still means a blanket yes for edits.

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
nix run ../dev-lint#gate -- . gate.json   # what the pre-commit hook runs
```

`gate.dhall` is the gate — thirteen named checks (cargo fmt/clippy/test, eslint,
the e2e typecheck, vitest, both application builds, both layout harnesses, the
graph-layout report, `memory-lint`, and the shared dev-lint rules), each reported
by name rather than as one `&&` chain. `gate.json` is rendered from it and
committed, so running the gate needs no `dhall`; one of the checks re-renders and
diffs the two.

`memory-lint` is the corpus's own static analysis (`lint.rs`, run by the memory
repo's pre-commit gate as well): frontmatter shape, dangling and untyped links,
paths that no longer exist, memories that are used together but never linked —
and reachability, which is the rule worth knowing: every memory must be walkable
from `MEMORY.md` through links, however many hops away, but need not be indexed
in it.
