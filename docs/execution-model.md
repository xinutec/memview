# Execution model

Design for the syntax layer under `reader/`: a faithful tree for every language
the fleet executes, plus a printer that puts it back.

**Status: simple commands, comments, pipelines, and-or lists, redirection and
tilde prefixes; both gates wired.**
`reader/src/syntax/` holds the tree, the parser and the printer; the
`bash-oracle` crate holds the second gate and the report.

**No coverage rate is tracked here** — each is one `cargo run` away and moves on
its own. The figures that stay are the ones that *sized a decision*, and they say
so where they appear.

```sh
cargo run --release -p bash-oracle --bin syntax-report -- \
  ~/.claude/corpus/union.jsonl --oracle [--why SUBSTRING]
```

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
because the command parses. Two defences, neither of them the law:

- **inside a word, by construction** — no run of literal characters may swallow
  one that opens a construct. Unquoted and inside double quotes those are `$`,
  `` ` ``, `\` and the quotes, so an unimplemented `${x:-y}` is a parse *error*
  rather than a literal. Nothing observes this after the fact; it has to be
  impossible.
- **across words, by the second gate** — bash's own printer, below.

## Second gate: bash's own printer

`declare -f` on a wrapped command makes bash print its parse. Measured against
bash 5.3 by `reader/probes/bash-printer.sh`, which is what every row below is
made of — re-run it against a new bash before trusting any of them:

| | |
| --- | --- |
| a fixpoint | printing its own print returns it unchanged, every shape tried |
| verbatim on words | `a`, `'a'`, `"a"`, `ec'h'o`, `${x:-y}` come back as written; only `$'…'`, `$"…"` and `\`-newline are resolved |
| normalising on structure | indentation, `;` versus newline, `f() { … }` → `function f () { … }`, a `( … )` function body wrapped in a brace group |
| desugaring | `ls \|& cat` → `ls 2>&1 \| cat`; `! time a \| b` → `time ! a \| b` |
| near-verbatim on heredoc bodies | reproduced as written, including one nested in `$( )` inside a double-quoted word — but a `\`-newline in an *unquoted* body is joined, and `<<-` strips its tabs, both at parse time |
| blind to comments | deleted |

So the comparison is **tree against tree** — parse the command, parse bash's
print of it, require the same tree, exclude comments. Not text against text:
bash preserves the spelling we normalise away.

⚠ **Bash is shown the ORIGINAL command, never our print of it.** A gate fed its
subject's own output can only confirm self-consistency; while it was, it caught
nothing, and a misparse of `a |⏎b` passed both gates. **The text that is misread
is the one text bash has to see.**

⚠ **It executes.** A balanced payload defeats the wrapper —
`echo a; }; touch X; { echo b` closes the function, runs, and reopens a group for
the trailing brace. What keeps such text out today is that the gate runs only on
*accepted* commands and the accepted language refuses `(`, `)`, `{` and `}`.
**That argument lapses the moment grouping is accepted, and then this needs
`sandbox-exec`** denying process execution and writes outside a scratch
directory.

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
a word at all: it is a field on the pipeline, and a *quoted* one is an ordinary
word the printer must quote to keep it one.

Neither gate catches a tree that confuses the two: bash prints the quotes
straight back, so a parser that collapsed them collapses them again and both
comparisons agree. Construction requirement, fixtures behind it.

### Grammar, not elevation

Two kinds of wrapping, and they belong at different layers.

| | examples | where |
| --- | --- | --- |
| shell grammar | `time`, `time -p`, `!`, `FOO=bar cmd` | the tree — fields on the pipeline or the simple command |
| commands taking a command | `timeout`, `nohup`, `env`, `nice`, `sudo`, `bash -c` | elevation |

`type -t time` says `keyword`, and the pipeline is `[time [-p]] [!] cmd [| cmd …]`.
Three things about it were measured rather than assumed, and each shapes the node:

- **Either order, one tree.** `! time a | b` comes back from `declare -f` as
  `time ! a | b`, so the tree holds two independent flags and the printer emits
  bash's order.
- **Head only.** `a | ! b` is a *syntax error* while `a | time b` is accepted and
  runs `/usr/bin/time`. So after a `|`, `!` is refused and `time` is an ordinary
  word — and the printer must quote a `time` at the head while leaving one after
  a pipe bare.
- **`!` is a toggle, not a count.** Bash prints `! ! a` back as `a`.

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

### A refusal ranking is not a work queue

⚠ **Never plan off the ranking.** The parser stops at the first construct it
cannot read, so each command is counted once, under whichever came leftmost —
which answers "what stopped us", not "what would building this unlock". Measured,
the top of the ranking was worth the least of the top three and the pipe was
worth 3.4× it. `syntax::survey` returns the whole set a command needs, and the
report plans off a greedy cumulative curve; its predictions have matched the
realised coverage at every step so far.

Most refused commands need three or more constructs, so a per-construct
percentage is the wrong unit.

⚠ **A reason is a unit of work, so split it the way the work splits.** `<` and
`>` began as one `Redirection`. Split into a file-or-descriptor target, a heredoc
whose operand is on the following lines, and a process substitution that is a
whole command, the file forms alone were four fifths of it — and the hard half
was never on the critical path.

The split of `ReservedWord` said it loudest. One bucket of 8,681 keywords was
**99.1% loops**: `for`/`while`/`until` block 8,606 commands and unlock 4,573 on
their own, while `if` blocks 75 and unlocks **50**. `case`, `[[ ]]`, `function`
and `coproc` block nothing at all in 131,246 commands — this corpus is
interactive commands, not scripts. Building the obvious "simplest compound"
first would have bought fifty commands.

The same split of `Expansion` said it twice more. Naming a parameter unlocks
5,795 commands and is a leaf; command substitution unlocks 2,273 and needs this
parser to recurse into itself; an operator inside the braces unlocks 508 and is a
language of its own. **The cheap form was worth 2.5× the expensive one**, and one
blended `Expansion` would have put the recursion first. Splitting also found that
a `$` opening nothing is an ordinary character — 616 commands were being refused
over a dollar sign that expands to itself.

⚠ **The survey is a second scanner, pinned rather than trusted.** It must read
text the parser cannot, so it cannot be built from the parser, and it drifted on
its first run. The invariant: the parser's refusal appears in the survey's set,
and the set is empty exactly when the parser accepts. The report re-checks it on
every corpus row and says so.

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

### Known misparse, fixed in the tree and still live in the reader

`time ./x.sh` parses in the flat reader as a simple command with
`argv[0] = "time"`, because it has `time` in its wrapper list beside `nohup` and
`exec`. That is a category error: `time` is grammar. It reaches the right command
anyway, so no count reports it. **Still unfixed there.**

The tree has the pipeline node now, so `time` and `!` are fields on it and the
scope question the flat model could not express — `time a | b` times the whole
pipeline — has an answer.

## What the construction rests on

Everything not modelled is refused **by name**, and the ranked refusals are the
work queue. Coverage starts low on purpose — a rate that begins high is a parser
absorbing what it does not understand.

Four properties are load-bearing and each has a test that fails without it:

- **`Span` compares equal to every other `Span`.** Position-blindness lives in
  the one type rather than in each node's `PartialEq`, so a node added later
  cannot forget and quietly make the law unsatisfiable.
- **The printer takes no source.** There is no `&str` of input in its
  signatures, which is what makes condition (2) a real check rather than a
  restatement of (1).
- **The printer quotes a word that would read back as grammar.** A tree holding
  the literal `time` as a command name must print `'time'`, or `t₂` is a
  different program.
- **No quote may touch a tilde prefix.** `~'/x'` is the literal `~/x` to bash, so
  the closing `/` goes through bare and quoting resumes after it. Found by the
  law on 319 commands.

⚠ **Only a reader that is not ours can object to a consistently wrong tree.** The
round-trip law is satisfied by *any* wrong answer that prints and re-reads as
itself, and the survey only knows what is refused. Gate 2 has caught two such —
a descriptor normalisation and a word split at a line continuation — each written
up at the node it corrected. It found nothing while it was fed our own output.

⚠ **`bash -n` adjudicates every refusal that is a claim about the input.**
`UnterminatedQuote`, `DanglingEscape` and `EmptyOperand` assert the text is not
shell; every other reason asserts only that we do not model something, which bash
has no opinion about. Checking them is what keeps "we cannot read it" apart from
"it does not parse" — the distinction above, which otherwise rots silently. A new
reason of that kind has to join the list, or it is an unchecked claim.

⚠ **"No gate can check it" is a reason to find a third check, not to refuse.**
A heredoc whose delimiter never appears looked un-judgeable: bash *accepts* it,
so the exit code says nothing, and `declare -f` cannot render it because the
runaway body swallows the wrapper's closing brace. It was refused on that
argument, and the argument was wrong twice over. **Gate 1 judges it perfectly
well** — the printer writes the terminator back, and `t₂ ≠ t₁` is permitted — and
**gate 2 already excludes a construct it cannot see**, namely comments. What was
missing was only a way to tell that exclusion from a defect, and bash's warning
(`here-document at line N delimited by end-of-file`) supplies it.

The corpus is shell history: those commands ran. Refusing them dropped real work
for a shape bash has a definite answer about. **Check which gate is actually
blind before concluding both are.**

⚠ **The oracle is its own crate.** `reader` states that it runs nothing and opens
nothing, and that claim is what lets the privileged console link it. Spawning
bash belongs outside it, for the same reason the console is a workspace member
and not a feature of the viewer.
