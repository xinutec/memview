# The reader

What the agents did, read out of the text they wrote, without running any of it.

The corpus memview serves is what sessions *wrote down*. The reader mines what
they **did**, from transcripts under `~/.claude/projects`.

Target: say what a command named and what it changed — which constant a name was
bound to, which files a literal loop touched, which machine a path belonged to,
which program a heredoc carried and what that did in turn. Exceptions should be
few, and each a known kind rather than a shrug.

It is an **abstract interpreter**: evaluate as far as the text determines, then
stop. What is undetermined is recorded as undetermined and counted, so the gap is
a number rather than a silence.

[execution-model.md](execution-model.md) specifies the syntax layer underneath,
`reader/src/syntax/`, and the two gates that govern it. The chain below is the
projection that layer will eventually be read through; today the two are
independent and are built from different grammars, so **a coverage figure from
one says nothing about the other**.

## Chain

Each stage's authoritative explanation is its module doc-comment.

| module | question |
| --- | --- |
| `reader/src/shell.rs` + `shell.pest` | which commands does this script run? |
| `reader/src/shell_ops.rs` | what does one command do, to which paths? |
| `reader/src/python.rs` + `python.pest` | same, for inline Python |
| `reader/src/shell_files.rs` | resolved against a cwd, which files? |
| `reader/src/activity.rs` | what kind of work — test, build, edit, deploy? |
| `reader/src/doing.rs` | timeline: agent · minute · repo · kind · count · verdict |
| `src/commits.rs` | what the repositories recorded, renames followed |
| `src/agents.rs` | who works where — the roster behind `/agents` |
| `src/couse.rs` | which memories are used together in one turn |

**The crate split is a boundary, not tidying.** *What does this text mean* lives
in `reader/`; *whose work was it* stays in the viewer. That lets the console —
which spawns processes on the root-of-truth Mac — read a command without linking
a viewer carrying routes, auth and configuration. `reader/src/lib.rs` has the
reasoning.

**Derived, never verbatim.** No command line, prompt or output text reaches any
artefact — typed structure and counts only. The rule and its one lifted half
(timelines, 2026-08-02) are in `doing.rs`.

## One path through the chain

```
cd health && nix develop -c bash -c "sed -i 's/a/b/' src/geo/velocity.ts"

shell.rs        two commands. The inner script is ONE WORD — quoting is the
                parser's job, meaning is not.
shell_ops.rs    ChangeDir{to:"health"} · Carries("-c") unwraps argv to `bash -c
                …` = Nested{script}, re-parsed in its own scope
                → Transform{program:"s/a/b/", in_place:true, paths:[…]}
shell_files.rs  cwd is …/health, so the relative path resolves; in_place makes
                it a write.
activity.rs     an edit.
```

Stops are deliberate. An unresolvable `cd` makes the cwd *unknown*, not stale, so
the relative path names nothing rather than the wrong thing. Under `ssh`,
everything belongs to that machine and never reaches the local index.

## Refusals

**An invented path makes every downstream count a lie.** Every refusal errs
toward undercounting.

- Nothing is looked up on disk — that filesystem is gone.
- Nothing is expanded beyond `~` and `$HOME`, the one knowable value.
- A word needs a `/`, a `~` or an extension to be a path. Costs real reads
  (`rg foo src` loses `src`); keeps flag values from becoming filenames.
- A remote path belongs to the remote machine.
- Both arms of an `if`, every arm of a `case`: *sometimes* reached. At most one
  ran.
- A function definition's body: *sometimes* reached. Defining it runs none of it.

**An undetermined subject is counted, not dropped.** Otherwise a command using a
file nobody can name is indistinguishable from one using none.

**A glob is not a shrug.** `for f in *.log` names no file — the directory it was
answered against is gone — but

```text
⟦*.log⟧  =  some S  ⊆  L(*.log) ∩ Files(dir, t)
```

is an unknown finite subset of a *known* language, which `$(git rev-parse HEAD)`
is not. Still counted as not named, since a subset of a pattern is not a file.
What it buys is falsifiability: the oracle runs the loop for real and asserts
every path bash touched matches the pattern (`S ⊆ L`), needing no old filesystem.

Bounded stops where the shell stops being concatenation. `${f%%:*}` is a rational
transduction and would need an automaton, so it stays opaque. The automata are
not built; the population needing composition is small enough to read off
`shell-files`.

**A loop body is certain only if the loop certainly ran it.** `while`/`until`
test before the first iteration, so empty input runs the body zero times. A `for`
is the other way: with `nullglob` off, a pattern matching nothing expands to
itself, so even a glob loop runs its body once. The rule is bash's.

## Method

1. **Pick the right corpus, and say which.** `~/.claude/corpus/union.jsonl` is
   the fixed denominator — comparisons across a change are only meaningful
   against it, because the live transcripts *shrink* and a rate rises when
   commands leave. Rebuild from transcripts when the question is about what is
   current. See the corpus discipline in
   [execution-model.md](execution-model.md).
2. Change the table or the grammar.
3. Re-run the report. The number that matters is the one that *moved*; the
   failure list is the next thing to build.
4. **Ablate.** Undo the change; the new test must fail.

⚠ **Read counts, not rates.** The denominator is commands *run*, not written, so
it moves whenever loop unrolling does and figures either side are not comparable.
It moves counter-intuitively: unrolling `$(seq N M)` adds commands and file uses
but opens loops holding commands the verb table lacks, so more is understood
while the rate falls.

⚠ **A parse rate is not a coverage figure.** A command that parses can hide the
ones inside it; `"$( … )"` was opaque while every report looked clean.

Two ways a figure has misled, each written up where it happened: a rate hiding a
trade (`reader/examples/tree-sitter-probe.rs`) and a census counting text already
read (`reader/src/bin/opacity.rs`).

## Running it

Mining is offline; `scripts/sync.sh` pushes the artefacts to the pod.

```sh
cargo run --release --bin agents        # → ~/.claude/agents.json + doing.json
cargo run --release --bin couse         # → ~/.claude/couse.json
cargo run --release --bin bash-corpus > /tmp/bash-corpus.jsonl   # what is current
```

The reports take any corpus file, so `~/.claude/corpus/union.jsonl` is the one to
pass when a figure has to be comparable with an earlier one.

⚠ Reports need `-p reader`; a plain `--bin` from the workspace root fails with
*no bin target in default-run packages*.

```sh
cargo run --release -p reader --bin shell-report     -- <corpus>  # grammar
cargo run --release -p reader --bin shell-files      -- <corpus>  # semantics
cargo run --release -p reader --bin activity-report  -- <corpus> [--sample KIND]
cargo run --release -p reader --bin python-report    -- <corpus> [--why|--sample]
cargo run --release -p reader --bin opacity          -- <corpus>  # what nothing reads
cargo run --release -p reader --example roundtrip-probe -- <corpus>
cargo run --release -p reader --example unparsed-probe  -- <corpus>

# the syntax tree: coverage, the ranked refusals, and both gates
cargo run --release -p bash-oracle --bin syntax-report -- <corpus> [--oracle] [--why SUBSTRING]
```

`--why <substring>` prints the commands behind every use of a matching path, and
settles arguments. `opacity` asks the inverse question: of the text these
commands carry, how much does nobody look inside, and who handed it over.

⚠ Run these rather than trusting any written-down number. Chaining with `; cat`
reports the `cat`'s exit code.

## Correctness

`reader/tests/oracle.rs` is the only test that catches a *wrong* reading rather
than a missing one. Shims go first on `PATH`, each logging its argv and cwd
before exec'ing the real tool, so the log is what bash did — globs expanded,
variables substituted, loops iterated.

Two properties:

- **sound** — never predicts a command the shell did not run;
- **exact where determined** — where the text determines everything, the
  prediction equals the log. Stops a reader passing the first by predicting
  nothing.

⚠ It runs only the fixtures in that file, in a scratch directory it removes. **No
corpus command is ever re-executed.**

## Not done

Each decided from a measurement kept with the thing it decided.

| not done | why | numbers |
| --- | --- | --- |
| a third-party parser | swapping loses more than it gains | `reader/examples/tree-sitter-probe.rs` |
| parsing regexes | biggest by volume; a regex names no file | `reader/src/bin/opacity.rs` |
| opening scripts on disk | what `deploy.sh` held *then* is unrecoverable | — |
| reading `node -e` | a query tool, not an editor | `reader/src/shell_ops.rs` |
| a policy-refusal channel | the refused bucket had one member | memview#820 |
| confirming `\|\|` from a non-zero exit | reachable by arithmetic, and tiny | `reader/src/shell.rs` |

The timeline is a separate thread: **Bash-only** (rows are pushed inside the
`Bash` branch of `agents::scan_transcript`, so `Read`, `Write`, `Edit`, `Grep`
and `Task` yield no activity), and it has no **episodes** — the grouping of rows
into stretches of one intent.
