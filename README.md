# memview

Two applications over the record a Claude Code fleet leaves behind, and the
crate underneath both.

**The viewer** — read-only web front end for the memory corpus: markdown with
YAML frontmatter, `[[wikilink]]` references and a curated `MEMORY.md` index.
Rust (axum) + Angular 22 Material. Live at `memview.xinutec.org`.

**The console** — front end for the *live* sessions on the Mac: start one, watch
it work, send it instructions.

**The reader** — works out what sessions *did*, from the commands in their
transcripts, without running any. Most of the fleet's file changes never pass
through `Write` or `Edit`, so counting only the tools that announce themselves
undercounts, and undercounts unevenly.

The console links nothing from the viewer. The viewer is read-only on an
internet-facing host; the console spawns subprocesses on the root-of-truth
machine. They share a repository, a toolchain and a gate, and nothing else.

## Documentation

| doc | what |
| --- | --- |
| [docs/viewer.md](docs/viewer.md) | routes, graph, auth, environment, deployment, gate |
| [docs/reader.md](docs/reader.md) | the reading chain, what it refuses, the reports |
| [docs/execution-model.md](docs/execution-model.md) | the syntax layer being built under the reader |
| [docs/agent-console.md](docs/agent-console.md) | the console's design and threat model |

Each stage's authoritative explanation is its module doc-comment; the docs carry
decisions and reasons, not a restatement of the code.

⚠ **No figures are written down here.** Coverage rates, corpus sizes and counts
move on their own and each is one `cargo run` away. The reports are the method —
see [docs/reader.md](docs/reader.md#method).

## Run

```sh
cd frontend && npm install && npm run build   # once, and after UI changes
MEMORY_DIR=~/.claude/projects/-Users-pippijn-Code/memory \
  STATIC_DIR=frontend/dist/memview-web/browser \
  nix develop -c cargo run
# → http://192.168.1.81:8091

./scripts/console-upgrade.sh              # build, install, move a RUNNING console onto it
nix run ../dev-lint#gate -- . gate.json   # what the pre-commit hook runs
```

⚠ **Restart the console with `console-upgrade.sh`, never `launchctl kickstart
-k`.** Kickstart sends SIGTERM, the console's deliberate *stop* path, taking
every open session with it. The upgrade signal is SIGUSR2 — `Roster::handover`
`execve`s the binary on the same pid, so the `claude` children never notice.
