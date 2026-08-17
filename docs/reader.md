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
`reader/src/syntax/`, and the three gates that govern it. **The chain below reads
through it** — `reader/src/project.rs` puts that tree into the flat shape the
rest of the chain consumes, and `project::read` is the entry point every artefact
is built from.

`shell.rs` and its pest grammar are still here, and are not dead: they are the
*second* answer that makes a disagreement mean something. `--bin projection` asks
both readers the same question over one corpus — **96.3% of 134,622 distinct
commands read identically**, and the 3.7% that do not are ranked. That comparison
is what found six defects nothing else could see; see *Two readers*, below.

## Chain

Each stage's authoritative explanation is its module doc-comment.

| module | question |
| --- | --- |
| `reader/src/syntax/` + `project.rs` | which commands does this script run? |
| `reader/src/shell.rs` + `shell.pest` | the same question, second answer — the check on the first |
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
# do the two readers agree about what ran? — and where they do not
cargo run --release -p reader --bin projection -- <corpus> [--show <n>] [--only <bucket>]

cargo run --release -p reader --example roundtrip-probe -- <corpus>
cargo run --release -p reader --example unparsed-probe  -- <corpus>

# the syntax tree: coverage, the ranked refusals, and all three gates
cargo run --release -p bash-oracle --bin syntax-report -- <corpus> [--oracle] [--why SUBSTRING]
```

`--why <substring>` prints the commands behind every use of a matching path, and
settles arguments. `opacity` asks the inverse question: of the text these
commands carry, how much does nobody look inside, and who handed it over.

⚠ Run these rather than trusting any written-down number. Chaining with `; cat`
reports the `cat`'s exit code.

## Two readers

`shell.rs` and `syntax/` answer the same first question — *which commands does
this script run* — from grammars that share no code, and for months nothing asked
them the same question. `project.rs` projects the tree onto `Simple`, the flat
reader's own shape, so `--bin projection` can diff them command by command.

⚠ **The differences were not evenly distributed, and neither reader owned them.**
Measured 2026-08-17 over `union.jsonl`:

| what disagreed | commands | who was wrong |
| --- | --- | --- |
| a backslash inside `" "` | 21,481 | flat reader — **fixed**, `unquote` |
| an argument before a redirection | 1,649 | flat reader — **fixed**, `shell.pest` |
| `do if x; then …` — the branch never opened | 621 | flat reader — **fixed**, `leading_keywords` |
| `a &&⏎b` — the newline ended the list | 86 | flat reader — **fixed**, `walk` |
| `cmd &>file` — the `&` was a separator | 18 | flat reader — **fixed**, `shell.pest` |
| `${n}_v4` printed as `$n_v4` | 8 | the projection — **fixed**, `print_value` |
| the condition on a loop body | 1,048 | flat reader, and **structurally**: see below |
| `<(cmd)` as an argument | 154 | flat reader: it is a redirection there |
| how an expansion is spelled back | ~2,600 | neither — see below |

Five of the six fixed ones were live defects nothing else could see. `grep -E
"\s+"` was read as `grep -E "s+"`; `nc -w3 host 25 2>&1` named no port; a
`rm -rf "$p"` inside `do if …; then` was recorded as having certainly run.
Agreement went **79.3% → 96.3%**.

⚠ **The 1,048 that are left are the argument for the port.** `a && for f in x;
do b; done` runs `b` only if `a` worked, and to this grammar `do` is a word after
a `;`, so the condition resets. Carrying it across would mean rebuilding the loop
structure the tree already has — which is what porting to the tree *is*.

### The whole chain, both ways

**The chain reads through the tree as of `43ae9fe`** — `project::read` at every
entry point that builds an artefact, including the nested `bash -c` payloads.
Before switching, the same corpus was run through the same semantics table both
ways behind a `--tree` flag; that flag is gone, because with nested scripts on
the tree a "grammar" column would no longer have been one. The standing
comparison is `--bin projection`, which asks both readers the same question a
layer earlier and needs neither of them switched.

The grammar column below was measured at `dc2fe2a`, the commit before the
switch. Measured 2026-08-17:

| | grammar | tree |
| --- | --- | --- |
| calls unparsed | 101 | **51** |
| commands understood | 97.8% | **97.9%** |
| file uses | 176,164 r · 30,583 w | **190,588 r · 32,039 w** |
| distinct paths | 34,523 | **34,635** |
| uses the outcome confirms | 185,235 | **194,702** |
| subjects a glob loop bounded | 407 | **427** |
| subjects not named | 4.3% | 4.3% |
| **nested scripts unread** | **98** | **405** |

**The tree reads more of the same corpus, and reads it as more certain.** More
loops are run out — the grammar had to find the `done` by counting keywords and
lost the ones it mis-parsed — so more bodies are real commands rather than a
`$f`, and a loop that certainly ran contributes uses the outcome can confirm.
With the nested payloads on the tree as well (`43ae9fe`), writes rise again to
33,179 and the table reads 98.0%.

⚠ **One figure moved the wrong way, and it is the last row.** The tree refuses
by name where the grammar guessed, and inside a `bash -c '…'` payload it refuses
four times as often — 405 scripts whose file uses are lost, against the 98 the
grammar lost. Every other column is better, which is why the switch happened
anyway; **what those 405 are has not been measured**, and until it is, "the tree
reads more" is true of the whole and unproven of that part. memview#1028.

⚠ **Two entry points, and the difference between them is the whole of the
evaluation.** `project` is what the script *says*: one command per command
written, comparable with `shell::parse`. `run_out` is what it *did*: loops the
text determines run out into their iterations, and a body that may have run zero
times demoted. Putting the zero-times rule in `project` cost two points of
agreement in the table above and was a false disagreement — it compared one
layer's decision against another's stage.

⚠ **A spelling difference is not a reading difference.** An argv string holding
`$(a|b)` is one reader's source text and the other's reprint of a parsed tree, so
`2>/dev/null` comes back as `2> /dev/null` and `${x}` as `$x`. Nothing reads
inside an expansion — its value is undetermined either way — so those buckets are
named apart rather than counted as defects, and `reader/tests/projection.rs`
asserts they stay spellings.

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
