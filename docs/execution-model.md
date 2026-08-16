# Execution model

Design for the syntax layer under `reader/`: a faithful tree for every language
the fleet executes, plus a printer that puts it back.

**Status: specified, not built.** No coverage rate is recorded here: each is one
`cargo run` away and moves on its own. Figures that size a decision stay.

## Purpose

Two questions per command, neither well answered today:

- **prediction** — what a command will name and change, before it runs;
- **reconstruction** — what it did, from history.

Diffing the two is the point.

The existing reader answers a weak form of the second, and what it *knows* is
worth carrying over — see Placement for what that means about its code. It has no
tree: output is a flat command list with structure projected away. So nothing
above it can render a command back, compare two commands for equivalence, or hold
an embedded language as a node rather than a substring re-parsed from the outer
text.

## The round-trip law

`P` parses, `G` generates.

```
t₁ ──P──→ A₁ ──G──→ t₂ ──P──→ A₂ ──G──→ t₃

(1)  A₂ = A₁          the generated text parses to the same tree
(2)  t₃ = t₂          the generated form is a fixpoint
     t₂ ≠ t₁          permitted: layout and quoting normalise
```

(2) follows from (1) when `G` is a pure function of the tree. It is stated
separately because the only way to satisfy (1) and fail (2) is a `G` reading
something else — the source buffer, a comment-offset table, a span. Printers are
commonly written that way. The constraint is therefore: **the tree is
sufficient.**

**Spans are retained and excluded from `PartialEq`.** They are needed for error
reporting and transcript pointers. `A₁` and `A₂` carry different spans, so a `G`
that reads one fails (2). Erasing spans to make equality easy makes the law
vacuous.

⚠ **The law cannot see a systematic misparse.** A parser treating an
unimplemented `${x:-y}` as a literal parses, prints faithfully, and re-parses to
the identical wrong tree; (1) and (2) both hold, and the corpus does not object
because the command parses. Hence a second gate, required from the first
construct: a subset parser is where unimplemented constructs get absorbed into
literals, and adding the gate later means re-auditing everything validated
without it.

## Second gate: an independent parse oracle

Bash prints its own parse — wrap a command in a function, `declare -f` it, and
bash renders its tree as text with no execution. Compare our canonical print
against bash's. The oracle is not ours, so it does not share our blind spots.
Needs its own layout-normalising comparison.

Distinct from `reader/tests/oracle.rs`, which shims `PATH` and diffs predictions
against real execution. That covers expansion, globbing and `cd`, and cannot
scale to the corpus — history is never re-executed. Two oracles, two jobs: shims
for semantics on fixtures, `declare -f` for parse shape on the corpus.

## What the tree holds

| retained | normalised away |
| --- | --- |
| comments — byte-exact, in a node with a slot for later semantic parsing | whitespace, indentation |
| every distinction that changes meaning | quoting style |
| spans, outside equality | blank lines, unless preserving costs one counter on the following item |

A word is a sequence of typed segments; quoting is derived at print time.

- `'a'` = `"a"` = `a` — one tree, one word.
- `"$x"` ≠ `$x` — they differ in splitting.
- `'$x'` is a literal, not an expansion.

So `t₂` is a canonical form: two textually different commands that mean the same
thing compare equal. **Non-destructive means the tree retains everything except
layout and quoting style.**

⚠ **The collapse rule stops at reserved words, where quoting is semantic.**
`time ./x.sh` runs bash's keyword; `'time' ./x.sh` runs `/usr/bin/time`, a
different program with different output. Same for `!`. So a reserved word is not
a word, the tree must record which it is, and the printer must never quote one.
`declare -f` catches this, because it preserves the quotes for the same reason.

### Grammar, not elevation

Two kinds of wrapping, and they belong at different layers.

| | examples | where |
| --- | --- | --- |
| shell grammar | `time`, `time -p`, `!`, `FOO=bar cmd` | the tree — fields on the pipeline or the simple command |
| commands taking a command | `timeout`, `nohup`, `env`, `nice`, `sudo`, `bash -c` | elevation |

`type -t time` says `keyword`, and the pipeline is `[!] [time [-p]] cmd [| cmd …]`.
**Scope is what forces it into the tree:** `time a | b` times the whole pipeline,
while `nohup a | b` applies to `a` alone, and `time` at `argv[0]` cannot express
the difference.

## No escape hatches

- **Nothing is left unparsed.** A payload in another language gets another
  parser. JSON, YAML and the rest are parsed as themselves.
- **Prose is a typed `Text` leaf** — a commit message, a task body. The leaf is a
  claim about the *site*, never a fallback from a failed parse.
- **Each parser covers 100% of the corpus, not 100% of the syntax.** Method
  unchanged: add what the failure report ranks highest, re-measure.
- **"We cannot parse it" and "it does not parse" stay distinguishable.** The
  corpus is self-labelling — a command that exited 0 was accepted by bash, so if
  we cannot read it we are wrong. `bash -n` is a second opinion and executes
  nothing.

## Embedding

Shell contains a word, not a Python node. The language follows from the
execution site: `python3 -c`, a heredoc fed to an interpreter, a
`nix-shell -i python3` shebang. `<<'PY'` names it; `<<EOF` does not.

- **Recognition is a separate pass** over a pure host parse, attaching
  `Embedded { lang, ast }`. Keeping it out of the parse means improving a
  recognition rule does not change the tree of unchanged text.
- **The law holds independently at each layer.**
- **Descend until no semantic layers remain** — shell → Python → shell → …
- **Expansion-bearing payloads are descended into as well.** `<<PY` with an
  unquoted delimiter is a shell word that expands into Python, so it parses as a
  program with holes. Tractability depends on where a hole lands: inside a string
  literal it is a token, across a token boundary it is not. Which shapes occur is
  a corpus question, not one to answer in advance.

## Measurement

Four numbers, reported apart — **per command**, **per byte**, **per node**, and
**depth**.

- The first three diverge: a parser at 99% of commands can be at 60% of bytes
  when the long commands are what fail. One blended figure hides that.
- **Depth** is share of code payloads descended into, and share of corpus bytes
  at layer ≥ 2. Without it a parser that descends into nothing scores 100% on the
  host layer. `reader/src/bin/opacity.rs` already asks a version of this.

⚠ A parse rate is not a coverage figure, and the trap recurs at every layer.

## The corpus

**Freeze a snapshot and measure against it.** The live corpus shrinks: Claude
Code prunes its own sessions, and since 2026-08-14 the odin archive mirrors
deletions rather than appending. odin's restic retention is the only deeper
history.

1. Commands leave the denominator, so coverage rises with no work done. A ratchet
   against a moving corpus is not a ratchet.
2. Loss is continuous, so the snapshot precedes the parser work.

**It lives in `~/.claude/corpus/`** — dated snapshots beside a cumulative
`union.jsonl`, written nightly by `xinutec-infra/scripts/claude-corpus-snapshot.sh`
from `claude-sync.sh`'s mining step, before the archive runs. It sits there to
inherit an existing durability path rather than grow a new one: `~/.claude` is
already rsynced to odin nightly and held in restic. Not in this repository —
memview is public and that is shell history.

⚠ **A run that stops happening has to be visible, or the ratchet rots quietly.**
`fleet_health`'s `claude corpus` check grades the snapshot's age hourly and
charts the union's row count, so a stalled job and a shrinking union both
surface. This is the one job here whose silent failure destroys something: what
it captures is being pruned from the live tree by design.

**Union for the ratchet, snapshot for frequency.** A corpus row is
`{cmd, cwd, ran}` with no call id, so `sort -u` collapses exact repeats the
moment it merges. Distinctness survives and multiplicity does not, which is why
the dated snapshots are kept beside the union rather than replaced by it.

**The first copy of a duplicated call wins.** Transcripts re-append stretches
already written, and the later copy carries a shallower `cwd`.
`src/bin/bash-corpus.rs` does this.

**No invented test input.** The corpus is the test suite. Hand-written cases come
from someone who knows a parser is watching; past sessions did not. Fixtures stay
correct for the semantics oracle, which cannot use history at all.

## Scope

Bash first, then the Python inside it — Python arrives constantly as a payload,
so the two do not stay separable.

`Bash` tool calls only. `Workflow` JavaScript and scripts checked into
repositories are deferred, not declined.

## Placement

The tree goes underneath the reader. **The existing reader is prototype
quality and is not a constraint on this design** — it may be evolved freely or
dropped. What is worth keeping from it is the *knowledge*: the semantics tables,
the refusals, and the tests, which make a regression corpus no rewrite has to
re-derive from scratch.

The ordering constraint is correctness, not preservation. Nothing is built in the
flat model to tide things over, because a half-representation invented now is one
to throw away later.

### Known misparse, left for the tree

`time ./x.sh` parses today as a simple command with `argv[0] = "time"`, because
the reader has `time` in its wrapper list beside `nohup` and `exec`. That is a
category error: `time` is grammar. It reaches the right command anyway, so no
count reports it, and it is 236 distinct corpus commands. **Not fixed in the flat
model** — a correct fix needs the pipeline node to hang the flag on.
