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
both readers the same question over one corpus — **96.3% of 145,219 distinct
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
| `reader/src/javascript.rs` + `javascript.pest` | same, for inline JavaScript |
| `reader/src/sql.rs` + `sql.pest` | same, for inline SQL — but in TABLES, not files |
| `reader/src/program.rs` | the types both carried readers answer in |
| `reader/src/shell_files.rs` | resolved against a cwd, which files? |
| `reader/src/reading.rs` | the whole corpus surveyed, as a value the report and both apps draw |
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

   ⚠ **Absolute figures in this file dated on or before 2026-08-23 were taken
   over a union that held one era TWICE** — 141,545 of 298,895 rows, collapsed
   on 2026-08-24 by memview#1130. The conclusions stand, because they rest on
   ratios and on before/after diffs of the same corpus, where the duplication
   cancels on both sides. The counts do not: they are inflated, and any
   "biggest first" ranking taken then was weighted toward whatever spanned the
   transition.

   The STANDING figures here — the opaque-shapes table and the current
   coverage — were re-taken on 2026-08-24. The dated before/after tables were
   deliberately NOT: an ablation is a record of what was run, and its two halves
   are only comparable to each other.
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

⚠ **A bucket is only as good as the guess that fills it, and a guess with no
test does not announce itself.** `opacity` sniffs the language of a heredoc body
from a mark; one line opening `import ` filed TypeScript, Kotlin, Swift and Lean
alike as Python, and 731 of the 1,154 bodies under that label were not Python.
Nothing looked wrong, because a census prints the same shape of number either
way. `--why <label>` opens a bucket and names the mark that filed each body.

Three ways a figure has misled, each written up where it happened: a rate hiding
a trade (`reader/examples/tree-sitter-probe.rs`), a census counting text already
read, and a census mislabelling what it counted (both `reader/src/bin/opacity.rs`).

## Running it

Mining is offline; `scripts/sync.sh` pushes the artefacts to the pod.

```sh
cargo run --release --bin agents        # → ~/.claude/agents.json + doing.json
cargo run --release --bin couse         # → ~/.claude/couse.json
cargo run --release --bin bash-corpus > /tmp/bash-corpus.jsonl   # what is current
cargo run --release -p reader --bin reading-json   # → ~/.claude/reading.json
```

**`reading-json` is the survey, mined rather than computed on request.** It
takes ~13 seconds over 146k commands — fine for a report somebody waits on,
wrong for a page — and the artefact is 8 kB, so both servers hold it in memory
and answer instantly. `claude-sync.sh` runs it in the nightly, **after the
corpus snapshot**, because it reads what that step writes: running it first
would describe last night's corpus and stamp it with tonight's mtime, a
staleness nothing downstream could detect.

It carries counts and command NAMES, never a command line — `effects.json` is
where verbatim text lives, and this file is small enough to embed in a page.

The reports take any corpus file, so `~/.claude/corpus/union.jsonl` is the one to
pass when a figure has to be comparable with an earlier one.

⚠ Reports need `-p reader`; a plain `--bin` from the workspace root fails with
*no bin target in default-run packages*.

```sh
cargo run --release -p reader --bin shell-report     -- <corpus>  # grammar
cargo run --release -p reader --bin shell-files      -- <corpus>  # semantics
cargo run --release -p reader --bin activity-report  -- <corpus> [--sample KIND]
cargo run --release -p reader --bin python-report    -- <corpus> [--why|--sample]
cargo run --release -p reader --bin javascript-report -- <corpus> [--why|--sample]
cargo run --release -p reader --bin opacity          -- <corpus> [--why|--dump <label>]
# what the SQL touched, and what it still cannot read
cargo run --release -p reader --example sql-corpus   -- <corpus>
# the parse tree for one SQL script, when a clause silently fails to match
cargo run --release -p reader --example sql-probe    -- "SELECT 1 FROM t"
# do the two readers agree about what ran? — and where they do not
cargo run --release -p reader --bin projection -- <corpus> [--show <n>] [--only <bucket>]

cargo run --release -p reader --example roundtrip-probe -- <corpus>
cargo run --release -p reader --example unparsed-probe  -- <corpus>
# which nested scripts will not read, by construct AND by who handed them over
cargo run --release -p reader --example nested-why           -- <corpus> [--reason NAME]
# what changed when a container payload stopped being joined into a script
cargo run --release -p reader --example remote-argv-check    -- <corpus>
# is there Python we never noticed was Python? — and does what we found parse?
cargo run --release -p reader --example python-calls          -- <corpus>
# does a real runtime agree about what never ran? (both readers)
cargo run --release -p reader --example python-raised         -- <corpus> [--all]
cargo run --release -p reader --example javascript-raised     -- <corpus> [--all]
cargo run --release -p reader --example tree-sitter-python-probe -- <corpus>

# the syntax tree: coverage, the ranked refusals, and all three gates
cargo run --release -p bash-oracle --bin syntax-report -- <corpus> [--oracle] [--why SUBSTRING]
```

`--why <substring>` prints the commands behind every use of a matching path, and
settles arguments. `opacity` asks the inverse question: of the text these
commands carry, how much does nobody look inside, and who handed it over.

⚠ Run these rather than trusting any written-down number. Chaining with `; cat`
reports the `cat`'s exit code.

## Two readers

**The chain reads through the tree; the pest grammar is kept as a second answer.**
`shell.rs` and `syntax/` answer the same first question — *which commands does
this script run* — from grammars that share no code, so each is an oracle for the
other. `project.rs` puts the tree into `Simple`, the flat reader's own shape, and
`--bin projection` diffs them command by command over one corpus.

⚠ **Keep both, and keep them apart.** The moment the flat grammar becomes a call
to the tree, nothing checks either of them again — which is why the switch was
made at the chain's entry points and not by redefining `shell::parse`. The
comparison is worth its keep: run once, it found six defects and five of them
were live, taking agreement from 79.3% to 96.3% — of 134,622 distinct commands
then, and the rate has held as the corpus grew past 145,000.
`grep -E "\s+"` had been read as `grep -E "s+"` in 21,481 of them. Each rule is
written at the code that carries it; the report ranks what is left.

**How to read a bucket.** A disagreement is not a defect until you look, and it
can belong to either side:

- **A spelling difference is not a reading difference.** An argv string holding
  `$(a|b)` is one reader's source text and the other's reprint of a parsed tree,
  so `2>/dev/null` comes back as `2> /dev/null` and `${x}` as `$x`. Nothing reads
  inside an expansion — its value is undetermined either way. Roughly 2,600
  commands, and `reader/tests/projection.rs` asserts they stay spellings.
- **One systematic spelling difference outnumbers every real misreading**, which
  is why the buckets are named apart and why `--only` exists.
- **The largest real one is structural**: 1,048 loop bodies whose `&&` condition
  the flat grammar cannot carry across the `;` before `do`, because `do` is a
  word to it. That gap is not fixable there — having the loop is what the tree is
  for.

⚠ **`project` is what a script *says*; `run_out` is what it *did*.** One command
per command written, against loops the text determines run out into their
iterations with a zero-times body demoted. The evaluation is the whole of the
difference. **Do not move the zero-times rule into `project`**: it is a statement
about running, the flat chain draws the line in the same place, and putting it
earlier reports a disagreement that is only a difference of stage.

⚠ **A flat outer parse cannot feed a tree nested one.** `shell.rs` hides a
heredoc body inside its own delimiter so a nested re-parse can still see it, and
only that reader decodes the marker — mixing the two silently loses every nested
`python3 - <<PY`. Read the outer and inner scripts with the same reader.

## What a payload is

**A payload is text only when a shell is on the far side.** `ssh host '…'` joins
its words and hands them over, because ssh really does; `bash -c` and
`nix-shell --run` take a script. `kubectl exec -- prog`, `docker exec`,
`subprocess.run([…])`, `spawnSync` and `execFileSync` reach `exec()` with no
shell — nothing re-splits their words and nothing removes a quote — so their
payload stays an **argv to classify**, never text to parse. That costs no second
parse and therefore cannot fail one, which is the same reason `Verb::Carries`
exists.

⚠ **Getting that wrong was 700 of the 769 nested refusals, and it looked like a
grammar gap for as long as nobody asked who carried the text.** The count is
ranked by the construct that stopped the parse, and at 813 occurrences 766 of
them were `Grouping` — unmatched `( )`. `nested-why` grew a second table, by the
command that HANDED the payload over, and **686 came from one shape:
`kubectl exec … -- <a program that is not a shell>`**. Joining those words back
into a string put SQL and JavaScript in front of the shell grammar, where
`SELECT ROW_COUNT() AS deleted` is unmatched grouping and so is every `node -e`
body with a `{` in it. Kept as an argv and classified: **813 → 73**.

**The rule that generalises: a refusal names the construct that stopped the
parse, never the reason the text was there.** Rank by carrier as well as by
construct.

The exchange was measured rather than assumed, because "fewer refusals" and
"fewer findings" can be one change seen twice. `--example remote-argv-check`
computes both readings of all 3,034 container payloads: same subjects for 3,026,
**four** paths gained that the old reading lost inside refused payloads, **one**
dropped that it had invented (`/p`, the tail of a `sed` address range the join
split at a `;`), and one recovered by teaching it that `su -s /bin/sh -c` names
its shell in a flag. memview#1028.

What is left refusing is 73, ranked as before; an earlier reading of that list —
when it stood at 405 — found 20 distinct scripts, 14 of which `bash -n` refuses
too, mostly a `\'` inside single quotes that ends the string early.

## The Python inside it

Three questions. **Does what we found parse?** `tree-sitter-python` reads 14,982
of 15,074 programs (99.4%).

**Did it run?** A different question, and the one that decides whether a file
operation is a fact. `python.pest` accepts a broken program as happily as a
working one and hands back the paths it mentions, which are then recorded as
work that happened — so `python::did_not_run` refuses the one shape confirmed to
raise, an escaped outer quote inside an f-string replacement field, and the
reader keeps nothing from it.

⚠ **The over-claim direction is the one that costs.** Flagging a program
discards everything it named, so a false positive destroys knowledge while a
false negative merely fails to gain any. `--example python-raised` hands every
program to real CPython and answers both ways: of 12,240 distinct programs
3.12.14 refuses 72, the reader flags 39, and it flags **nothing CPython
accepts**. Of the 33 it lets through, 19 hold an unexpanded shell variable and
ran perfectly well once the shell substituted it — two rules meeting, not a
defect — and 14 are genuinely broken, mostly a heredoc cut short.

⚠ **And `tree-sitter` is not the authority for this.** It accepts 10 of the
programs CPython refuses; an editor's parser is built to keep going. An earlier
version of the rule fired on any backslash in a replacement field and threw away
two programs that worked, because PEP 701 permits `f"{'\n'.join(x)}"` on 3.12.
Ask the interpreter that ran the corpus.

**And is there Python we never noticed was Python?** That one a coverage figure
cannot answer, because its denominator is what we found — a call the verb table
does not recognise is *absent* rather than wrong. `--example python-calls` asks
it from outside the table, by the shape of the argv: **17,453 commands name an
interpreter and none is missed.** It found one gap, `python3.12`, which matched
neither literal spelling the table carried.

⚠ **That probe must stay looser than `shell_ops::is_python`.** It is the
instrument and the table is what it measures; if it asked the table's own
question it could never find a spelling the table has not been taught. Which is
also why `python313` — a nixpkgs attribute in `nix-shell -p python313`, 68 of
them — is matched and then shown to run nothing, rather than filtered out early.

## The JavaScript inside it

The third language, added 2026-08-22, and it is `python.rs` with the nouns
changed: a grammar for the syntax, a module for the meaning, a report ranking
what it could not read. The types both readers answer in are shared
(`program.rs`) so a figure from one can be read against a figure from the other.

**It is here because the ranking that kept it out was measured on the wrong
denominator.** The "not done" row said `node -e` is a query tool, not an editor,
on 724 calls with 23 writes. Counted again over the whole corpus: **11,748 Bash
calls name a JavaScript runtime, 3,824 carry a program in a flag**, and inside
them are 1,790 `readFileSync`, 1,909 `require`, 670 `import` and 214
`writeFileSync`. A projection is mostly about reads, and reads had not been
counted at all. Standing now: 2,306 programs read, 4,347 file operations, 86.0%
of them naming a file, 1,907 uses kept as paths.

Three things Python does not have had to be answered.

⚠ **A regex literal and a division are the same character**, and position is
what tells them apart: a `/` where an atom is expected cannot be division,
because division needs a left operand. The grammar gets this for free by trying
`regex` only in atom position. `did_not_run` does not, and its first version
read the `'` inside `.replace(/['"]/g, "")` as an opening quote, declared the
string unterminated, and **threw the whole program away** — the direction that
destroys knowledge. It now tracks the last significant character and applies the
same rule. Caught by a test, not by the corpus.

⚠ **An expression may cross a newline**, which in Python it may not. This corpus
writes `raw.filter(…)⏎  .map(…)`, and a grammar that ends the statement at the
line break turns `.map` into a new one — which is how `map`, `filter` and `then`
appeared on the worklist. Newlines are allowed before a `.` and inside brackets,
and **nowhere else**: at statement level `p = 'a'` and `q = 'b'` on two lines
would otherwise read as one value of two operands, and a value of two operands
is computed, so every constant in the program would quietly stop being one.

⚠ **A bare module specifier is a package, not a file.** `require("@angular/
compiler")` has a slash in it, so it passes any "looks like a path" test ever
written — and it names nothing on disk. Node's own rule is used: a specifier is
a path when it starts with `./`, `../`, `/` or `~`. Before that rule, 1,126 of
the recorded uses were the string `fs`.

**Did it run?** The same question the Python reader answers, checked the same
way. `javascript::did_not_run` refuses text whose quotes, template or brackets
never close — a heredoc cut short — and the reader keeps nothing from it, so a
false positive destroys knowledge while a false negative merely fails to gain
any. `--example javascript-raised` hands the programs to this repository's
pinned node: **the reader discards 2 and node refuses both**, and of the 1,025
it keeps node accepts 1,016. Seven of the nine refusals are TypeScript run by
`tsx`, which node cannot check and which the grammar handles by design; one
holds an unexpanded shell variable and therefore ran perfectly well; one is
genuinely broken.

## The SQL inside it

The fourth language, added 2026-08-23, and **the first that names something
other than a file**. `python.rs` and `javascript.rs` answer *which paths did this
touch*; this answers *which tables did it read, and which did it change*.

⚠ **It contributes nothing to any file count, and that is a measurement.** Over
5,727 corpus commands carrying a SQL client there is no `INTO OUTFILE`, no
`LOAD DATA INFILE` and no sqlite `.read`/`.output` — SQL in this corpus names a
file exactly never. Those forms are read anyway, a rule apiece, because the cost
of missing one is a write nobody sees. But a table is not a file, and folding
2,747 table reads into `reads` would be 2,747 files that do not exist.

Standing: **2,434 commands carried statements, 2,747 table reads, 264 changes,
151 distinct tables.** The 28 that carried statements and yielded nothing all
hold `$1` — passed in at runtime, so unknowable rather than unread.

Two entries in the table were wrong before this existed, and neither was
visible as wrong:

- **`sqlite3` was an `Interpreter`**, so it read its DATABASE as a script it
  ran: the right file, recorded as the wrong kind of use, and its statements
  never read.
- **`mariadb`/`mysql`/`psql` were `NoFiles`**, filed under "build tools that
  take targets". True of the operand — `mariadb health` is a database name on a
  server, not a path — and false of everything they carry.

⚠ **The direction of a table is decided by the VERB, never by the clause.**
`SELECT … FROM x` reads `x`; `DELETE FROM x` empties it. A reader mapping `FROM`
to "read" would report every deletion in the corpus as a lookup. The same rule
settles the sqlite database file: `sqlite3 x.db 'SELECT …'` and
`sqlite3 x.db 'DELETE …'` are identical in argv shape, so the file's direction
comes from parsing the payload.

⚠ **A table name and a function call are the same shape**, which is why this is
a grammar and not a regular expression. `FROM datetime(ts, 'unixepoch')` names
no table; a regex for `FROM (\w+)` answers `datetime`. Measured against a real
one: the regex reported **291 distinct tables where the grammar finds 151** —
about half its answer was fabricated. `qualified` refuses a name followed by
`(`, which is a decision a scanner cannot make.

⚠ **Four separate rules were written wrong the same way**, and the corpus caught
what tests did not. In a NON-atomic pest rule, implicit whitespace is inserted
between every pair of expressions — so `^"from" ~ !ident_char` skipped the space
and tested the table name, and no clause matched at all. Then `qualified ~ !"("`,
then an explicit `WHITESPACE+` that had nothing left to match, then a
`dot_command` that ate across the newline and swallowed 50 scripts whole. **Any
rule whose meaning depends on adjacency has to be atomic.**

⚠ **Nine tests failed and four passed, and the four were the tell.** All four
were negative assertions — *a function is not a table*, *a stream is not a file*
— which a reader recognising nothing satisfies perfectly. Had the suite held
only the guards against fabricated subjects, which is what the module is *for*,
it would have been entirely green over a reader that did nothing.

## The loop between them

A carried program that runs a command runs a shell's worth of work, and until
2026-08-22 all of it was invisible. **`subprocess.run` was the largest single
thing either carried reader could not read** — 443 calls, twice the next entry
on the Python worklist. `Program::ran` now carries what a program handed to the
system, and `shell_files::carried` follows it at the shell's own directory, so
`bash -c 'python3 -c "os.system(…)"'` reads all the way down and what comes back
may be another Python or JavaScript program in turn.

Which of the two a call hands over is decided the way *What a payload is* says —
by the call, never by the shape of the argument. One case only this side has:
**`subprocess.run("ls -la")` without `shell=True` is neither script nor argv.**
Python looks for a program of that whole name and fails, so reading it as a
script would credit the program with work it did not do.

⚠ **One unknown word makes a whole argv unusable.** `["ffmpeg", "-i", f]` with
`f` computed would otherwise read as an ffmpeg call over a file named `-i`.

What it bought, measured across the corpus: file uses 408,348 → 410,063 reads
and 71,359 → 71,497 writes, and 95 paths nothing had named before.

## Growing the table

⚠ **"Not understood" and "touches no file" are different claims, and the
worklist is only as good as the difference.** On 2026-08-22 the unread list was
headed by `task` at 12,761 calls — three times the next entry — which is the work
queue's own CLI, a store behind a server with no flag that names a file. It and
seven others (`ping`, `dig`, `nc`, `journalctl`, `dmesg`, `nixos-version`,
`mariadb-admin`) were checked one at a time against how the corpus actually calls
them and moved to `Verb::NoFiles`: **24,490 executions off the unread list**,
which is the sum of those commands' own counts — the check that the drop is what
was classified and not something else moving underneath.

The same pass taught four commands that do real file work and were contributing
nothing: `ffmpeg` (368 calls of the recall pipeline's audio), `ffprobe`, `unzip`
and `zstd` — **+524 reads, +110 writes, 88 paths nothing had named**. `ffmpeg` is
the one shape here where the output is positional and the inputs are not, and
**the path guard is what makes "the last operand" safe**: `-f null -` ends in a
dash, a probe ends in a number left over from an undeclared flag, and neither is
a path, so neither becomes a write. An archive's members are the mirror image —
`unzip x.zip 'FS/data/**' -d out` names something *inside* the zip, which passes
every path test ever written and is not a file on this machine.

⚠ **Checked one at a time, and `screen` is why.** It sits in the same part of
that list and 222 of its calls are `-X hardcopy /tmp/…`, a real file written, so
sweeping the neighbourhood would have deleted them silently — `Op::Nothing` is a
claim nothing downstream re-examines. It writes what `hardcopy` and `-Logfile`
name and nothing else. `journalctl` went the other way for the same reason:
`--file` exists, and this corpus never uses it.

`dhall-to-json` (444) and `lean` (377) came with it. The converter is the shape
worth naming: **both of its ends are flag values**, so unlike ffmpeg it has no
operand at all — and this repository's own gate is a caller, `gate.dhall` the
source and `gate.json` generated from it, a dependency that appeared in no
projection until now.

`md5` (323 calls), `openssl` and `wg` closed the same pass: the first two read
what they are given, and `openssl x509 -noout -enddate` reads
from a *pipe*, where the guard is what stops `x509` becoming a filename. `wg` is
an interface in all 371 of its calls — ⚠ `wg setconf <file>` would read one and
does not appear, which is where this would go wrong quietly. **Understood stood
at 99.2%, and the unread list topped out at 1,215.**

### The four the corpus could not answer, 2026-08-23 (memview#1070)

That pass stopped where the corpus stopped: what was left was a container runtime
and four of this fleet's own binaries, and the ticket recorded that their file
behaviour lives *in their sources* rather than in how they are called. **That
turned out to be false for three of the four, and the fourth is not a binary.**

`--example unread-shapes` was written to settle it — the population comes from
`Extract::unhandled`, the same field the rank is built from, and only the TEXT is
printed. ⚠ **A hand-rolled `grep '^lares'` was tried first and disagreed with the
reader by 2x in BOTH directions**: it counted the name inside grep patterns and
inside pasted Rust, and missed calls inside `bash -c` payloads. A worklist read
off the wrong instrument sends the work to the wrong place.

| | calls | what settled it |
| --- | ---: | --- |
| `ss` | 294 | 20 spellings, all flags over a socket table. `Verb::NoFiles`, beside `wg`. |
| `mysqladmin` | 284 | 5 spellings, all `ping`. `mariadb-admin` under its old name. |
| `verified_cli` | 336 | its **source**: every arg is a subcommand, data via stdin |
| `replay` | 1,076 | its **source**, which CORRECTED the call shapes |

⚠ **`replay` is the case for reading a source, and the case against reading only
calls.** `replay --words <dir>` reads as a valued flag; it is not one.
`scanner/server/src/bin/replay.rs` shows `--words`, `--slots`, `--tables`,
`--pdf`, `--paper` and `--bands` are bare mode tests and the session directory is
the only positional — with `--page N` the single valued flag, whose `2` would
otherwise have resolved into a file that nothing touched. So a binary's own
source **does** count as evidence here, and the reason is not convenience: it is
the only evidence that disagreed with the guess.

**What it bought:** understood 99.2% → **99.3%**, unread 20,073 → 18,083 — a drop
of exactly 1,990, which is the sum of the four commands' own counts. **+721
reads** and 28 paths nothing had named, all of them `replay`'s session
directories; `replay` had 411 writes recorded against it from redirections and
zero reads, which is the shape of a tool the table half-knew.

**Two were declined, with the measurement.** `k3s` (1,215) carries
`crictl exec "$C"`, whose target is a variable — an entry would move calls from
*unread* to *read nothing*, which is not progress. `lares` (378) was a Mac-side
Rust CLI **deleted** in its own repo (`~/Archive/lares`, commit `4a7968d`, "delete
Mac-side rust/python/web"); the calls are real history, but the evidence is now
git archaeology on a tool that no longer exists.

⚠ **`probe` (418) is not a binary at all — it is a shell function**, and all four
of its distinct spellings are `probe() {`. See below: it was one of seventy-eight.

### A call is not a gap when the script declares the function (memview#1124)

`probe` was not the case, it was the *symptom*. Measured the same day by
`--example defined-here`, then again by the reader itself: **2,475 calls across
77 names**, which was the largest single category on the worklist — bigger than
`k3s`. Every one of them work nobody could do: `r`, `p`, `A`, `maxid`, `render`
mean something different in every script that declares them.

⚠ **It cannot be a name list, and the numbers say so.** `check` is a real program
in `~/Code/check` AND a local helper — 112 of its 114 calls are declared in their
own text and 2 are not. `run` splits 185/10 the same way. The question has to be
asked of each command text, which is why `Ran::defines` is a property of the
SCRIPT and `shell_ops::classify` is not asked at all: from the call alone —
same word, same argv — it is not decidable.

**A third outcome, not a reclassification into either existing one.**
`Extract::local` is neither `unhandled` (there is no entry to write) nor
`handled` (the reader did not follow the call, and what it passes as arguments
goes unread). Both neighbours would have been a claim.

| | before | after |
| --- | ---: | ---: |
| not in the table | 20,073 | **15,608** |
| a local function | — | **2,475** (77 names) |
| understood | 99.3% | **99.3%** |

⚠ **The rate is unchanged, and that is the check.** `local` is in the
denominator of `understood()` and not in the numerator, so moving 2,475 calls out
of `unhandled` cannot raise it — nothing more was read. Verified by diffing the
report: `understood 2428358 (99.3%)` is byte-identical across the change, and
only the two lines below it moved. A coverage figure that improves because calls
left the denominator is a different fact from one that improves because the
reader learned something.

**What it bought is a worklist that is true.** With 2,475 phantom entries gone,
`arp` (167), `claude` (164) and `magick` (160) surface — real commands that were
buried under helpers nobody can teach.

⚠ **That table is an ABLATION and its counts are over the pre-#1130 corpus**,
which held one era twice. It is left as measured, because a before/after diff of
the same corpus is exactly where the duplication cancels — rewriting the numbers
would falsify the record of what was run. The standing figure today is **1,261
local calls across 78 names**, and `understood` is still 99.3%: the change
survived the corpus being halved under it, which is what a ratio is for.

⚠ **Still unread, and named rather than hidden: what the CALL passes.**
`render "$src" out.svg` binds `$1` and `$2` in the body, and the body's file uses
are recorded at the *declaration* under `Reached::Sometimes` — with the arguments
unbound. Reading them needs argument binding, which is why this pass stops at
making the number true.

⚠ **`md5` was the macOS spelling of something already in the table**, and adding
"the family" duplicated five entries the `Verb::Read` list has always had —
`md5sum`, `shasum`, `sha1sum`, `sha256sum`, `cksum`. **Read the list before
extending it**; `rustc` will say so, but only if something asks it.

The gate said so under `-D warnings` and the pre-commit check in that session did
not. ⚠ **Run clippy the way the gate does — `-- -D warnings` — and read its exit
status.** Grepping its output for the word `warning` cannot tell "clean" from
"did not run", and without `-D warnings` it exits 0 either way, so no exit-code
test bolted onto that form would help.

The tidy explanation for the miss — that cargo had cached the diagnostic — was
tested and is **false**: the same pipeline reports it on a fresh crate and a
stale one alike. The miss itself stays unreproduced, which is worth more than a
plausible story about it.

What is left there is a different kind of thing and is not guessable from the
corpus: `k3s` (1,215) carries a command, and `replay`, `probe`, `lares` and
`verified_cli` (2,208 between them) are this fleet's own binaries, whose file
behaviour is in their sources rather than in how they are called.

⚠ **A wrong reading costs more than a missing one, and `perl -pi -e` was both.**
It was read as an interpreter running a script, and it is a rewriter changing a
file. Both halves were wrong at once: the direction (a read recorded against a
file being written) and the kind (a script *run*, where the operand is a source
file being *edited*). It is the corpus's fourth
commonest command shape — `-0pi -e` 2,114 calls, `-pi -e` 1,028, `-i -pe` 38 —
and correcting it moved **4,044 uses from read to write**, "run a script" 53,621
→ 49,663 and "transform (in place)" 1,729 → 5,473.

Two things it turned on, both of which read as details until they were not:

- **`-i` is written in a cluster.** `-0pi` is `-0`, `-p` and `-i`, and the test
  was `starts_with("-i")`, which matches none of the 3,142 calls that spell it
  that way. The cluster test `Verb::Remove` already used for `-r` is the one
  that was needed.
- **A program in a flag is still a program.** With the text in `-e`, the
  `Op::Transform` was built with an empty `program`, so the opacity census —
  which decides what to read next — held no perl at all. "transform program,
  rewriting" went 137 kB → 751 kB when that was fixed, without a single new
  call being read.

⚠ **`ruby`, `deno` and `bun` were checked at the same time and left alone**, and
that is a measurement rather than a shrug: `ruby -e` appears **zero** times in
this corpus, `deno eval`/`deno run` zero, `bun -e`/`bun run` zero. `perl script.pl`
is zero too — the 288 apparent cases are all `nix-shell -p perl …`, where the
word is a package name.

## One survey, three views

`reader/src/reading.rs` holds the accumulation; `--bin shell-files` prints it,
`/api/reading` serves it to both apps, and the two views draw it.

⚠ **The extraction exists because a SECOND consumer is where a duplicated
calculation stops being free.** The survey was thirty mutable locals inside the
report's `main`, which is fine while nothing else asks — and the moment the API
asks, the alternative is two answers to *how much is understood* that drift
apart with nothing to say so. The refactor was checked by diffing the report's
output before and against after: **byte-identical across all 209 lines.**

| view | where | shows |
| --- | --- | --- |
| the report | `--bin shell-files` | everything, ranked, for somebody reading a terminal |
| `/reader` | the viewer | coverage, the verb histogram, files, databases, the fleet, the work queue |
| `/reader` | the console, behind the burger menu | the headline pair and every shape, on its own screen |

⚠ **Coverage and its ceiling are drawn together, on both.** 99.2% of commands
understood beside 4.5% of file uses whose subject the text cannot determine —
different denominators, commands against uses, which is exactly why neither can
stand in for the other. A page showing the first alone is advertising. For the
same reason `not understood` is a bar INSIDE the histogram rather than a note
under it: it is the size of the hole in that very chart.

⚠ **A 404 says "not mined here" rather than drawing zeroes.** "Nothing has been
mined" and "the survey found nothing" are different claims, and a default-filled
summary states the second. The console's screen says it in one muted line, which
is the correction to a first version that drew nothing at all — a component that
vanishes is indistinguishable from one that was deleted.

⚠ **It is a screen you go to, not a strip in the way.** The survey sat above the
session list, where it was the first thing read and the least often needed; it
moved behind the burger menu on 2026-08-23. Given its own screen it stopped
having to fit, so it shows every shape rather than the five that fitted.

## The space a subject lives in

What the text fixes about a value it cannot name. **A located language** — a
language `L` paired with a locus `D`, the directory the answer must live in —
which is one object with two halves, and the shapes below differ only in which
half is precise.

```text
⟦*.log⟧  =  some S  ⊆  L(*.log) ∩ Files(dir, t)
```

An unknown finite subset of a **known** language: cardinality unknown, possibly
empty under `nullglob`, possibly the pattern itself where nothing matched.
`bounded_by()` resolves the pattern against the cwd, so what is recorded is
`/abs/dir/*.log` and not `*.log` — **`*`, `../*` and `/*` are three different
spaces, not one shrug.**

Measured by `--example opaque-shapes` over the union corpus, 2026-08-24 — the
first reading after memview#1130 collapsed the duplicated era — of the 4,566
subjects the reader could not name:

| uses | shape | `L` | `D` |
| ---: | --- | :-: | :-: |
| 2,513 | a bare name, bound outside the text | — | — |
| 612 | locus known, leaf unknown — `Verified/Geo/${s%%:*}` | — | ✓ |
| 338 | derived — `${f%.ts}.js`, `$(basename …)` | ✓* | * |
| 338 | an environment directory — `$TMPDIR` | — | ~ |
| 314 | unclassified | — | — |
| 273 | a located finite set — `$(find $d -name '*.ts')` | ✓ | ✓ |
| 143 | a substitution with no locus | — | — |
| 29 | a positional parameter | — | — |
| 6 | **arithmetic — never a path** | | |

Plus the 490 already bounded, which have both. Of the 4,560 that are genuinely
path subjects: **a locus is known for 19.4%, a language for 26.8%.**

⚠ **Every count in that table roughly halved on 2026-08-24 and the two RATES
did not move** — 19.0% → 19.4% and 26.7% → 26.8%, against counts that fell 48%.
Nothing was learned between those readings: the corpus stopped holding one era
twice, so there was less to count and no more known. It is the cleanest
demonstration in this file of why a moved rate has to say which half moved, and
the reason the shape of the table is the durable part of it and the counts
are not.

⚠ **Two of those rows moved for opposite reasons on 2026-08-23, and telling them
apart is the whole discipline** ([[feedback_a_threshold_carries_its_denominator]],
memview#1079). The arithmetic row fell 458 → 12 and a row of 67 program bodies
went to zero because the READER stopped offering non-paths as subjects — the
denominator was wrong. The locus rate rose 17.7% → 19.0% because the CENSUS
learned to read a locus it already had — the numerator was wrong. Only the second
is more understood; the first is less overstated. Neither is progress on the
reader's actual limit, which is the bare-name row — 4,886 when this was written,
2,513 over the corpus of 2026-08-24, and the same fraction of the whole either
way.

The census had required the variable to be in the **leaf** — split at the last
`/`, and the directory literal throughout — so `Code/$p/node_modules` (56 uses,
the largest single shape in the unclassified bucket) counted as having no locus,
though `Code` is exactly where the answer must live. It now takes the literal
text ahead of the first `$`. Guarded on whitespace, because a one-line jq filter
carries a `/` and a `$` too, and widening without that would move a program
fragment into `locus known` — the direction that flatters the census.

⚠ **Regular is the right lattice because the class is closed under what shells
do to paths.** `dirname`, `basename`, `${f%.ts}.js`, `${f%%:*}` each map a
regular language to a regular language, so a derived value keeps a describable
space instead of collapsing. That closure is what makes `S ⊆ L` compose, and
composition is what makes it checkable.

⚠ **Past and future are the same object with a different `t`.** For history `L`
and `D` are exact and only `t` is gone — so the honest output was always the
*space*. For a command about to run, `t` is now, so `S = L ∩ Files(D, now)` is one
directory read. That is the *prediction* half
[execution-model.md](execution-model.md) names, and it must live above this
library — `reader` touches no filesystem, and that property is worth more than
the convenience.

### The locus is printed now, not just the pattern (memview#1080)

`Extract::located` records the directory a subject is rooted at when the text
writes one out ahead of the first expansion. **611 uses over 89 distinct loci**,
2026-08-24 — the census's `locus known, leaf unknown` row, moved out of the
shrug and into an answer.

```text
⟦Verified/Geo/${s%%:*}⟧  =  some path rooted at /abs/Verified/Geo
```

**The same object as `bounded` with the language half missing.** A glob gives
`L` and `D`; this gives `D` alone, because `${s%%:*}` is a transduction the
reader will not build an automaton for. A glob bound is therefore tried FIRST
and wins — filing a bounded subject as merely located would throw away the half
that makes it falsifiable.

⚠ **ROOTED AT, not contained in, and the difference is a `..` nobody can see.**
The word resolves from that directory, so a real run touches something under it
*unless the expansion climbs out*. An absolute-looking expansion does not escape
— `a/b//c` is `a/b/c` — only `..` does. Stated that way it stays falsifiable in
the direction that matters, exactly as `S ⊆ L` is for globs.

| | before | after |
| --- | ---: | ---: |
| shell, by word | 4,566 | **3,705** |
| shell, located | — | **861** (128 loci) |
| subjects not named | 11,974 | **11,974** |

Two shapes reach it. A **written-out directory** ahead of the first expansion,
611 uses; and a **finite-set generator** over a directory the text names —
`$(find . -name '*.ts')`, `$(git ls-files)` — 250 more.

⚠ **The generator is tested before the whitespace guard, because a generator is
nothing but whitespace.** That guard exists to keep one-line jq filters out, and
it would have thrown every `find` away with them.

⚠ **`git ls-files` and `git diff` look alike and are not one rule.** `ls-files`
with no pathspec lists what is tracked at or below the working directory,
printed relative to it, so the cwd is its locus. `git diff --name-only` and
`git status` print relative to the REPOSITORY ROOT wherever they run, and this
reader does not know where that is. One rule for both would root a use at a
directory it never walked. Measured: 251 located sets have a nameable
directory, and this refuses one of them on purpose.

⚠ **The not-named total is byte-identical, and that is the check.** Nothing more
was *named*; the reader says more about the same subjects. A locus is a better
answer than a shrug and is not an answer, so it stays in the denominator for the
same reason `bounded` and `local` do.

⚠ **The census's own row had to be repaired in the same commit.**
`--example opaque-shapes` takes its population from `by_word`, so those 611 left
it — its `locus known` row now reads 1, and its locus RATE would have fallen on
the day the gap was closed had `by_locus` not been added back to both the
numerators and the denominator. Verified by diffing the whole census: every
summary line is byte-identical, `885 (19.4%)` and `1,223 (26.8%)` included.

**What is left in the census:** one `locus known` — `amun:~/Photos/$f`, a remote
path whose directory is on another machine — and 23 `located finite set`, being
22 that walk a computed directory (`$(find $d …)`) and the one `git diff`. All
three shapes have a test, because they are what somebody reading those rows will
try to "fix", and each fix would name a directory the command never walked.

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

### Why this is not sound abstract interpretation

Asked and settled twice — Pippijn 2026-08-13, and again 2026-08-23. **Do not
"fix" the reader into soundness.**

Sound AI over-approximates so a property holds for *all* inputs: γ(abstract) ⊇
concrete, joins at merge points, widening at loops, and every undetermined value
becomes ⊤. Applied here, `rm "$f"` with `$f` undetermined has exactly one sound
answer: **may have deleted any file.**

⚠ **The accurate statement is not "the reader is unsound".** It is sound where it
speaks — `S ⊆ L` *is* an over-approximation — and it refuses to say ⊤ where
soundness would require it. The artefact is therefore **three parts, not one
bound**: a **lower bound** (what was named), a **described middle** (`S ⊆ L`,
sound and falsifiable), and a **counted remainder** (23,309, printed rather than
hidden). That is strictly more informative than either bound alone.

Four reasons, in increasing order of force:

1. **The question is about one trace, not all traces.** Over-approximation exists
   for "for every possible input". Exactly one execution happened and its inputs
   are mostly *in the text*, so γ is a singleton nearly everywhere; widening
   discards information that is present.
2. **⊤ absorbs, and aggregation is the product.** Every artefact joins over
   commands — what a session touched, what an agent changed in July. Join
   anything with ⊤ and the result is ⊤. With 20,073 commands not in the table
   and 102 unparsed calls, one anywhere in a turn makes that turn's answer *may
   have touched any file*, and the timeline becomes rows of identical nothing.
3. **The error would run in the expensive direction.** An undercount fails to
   gain; a fabrication corrupts every count downstream. ⊤ is the maximal wrong
   reading — it asserts files that were never touched.
4. **It would be unfalsifiable, which is disqualifying here.** `S ⊆ L` is
   checkable, and `oracle.rs` above checks it: run the fixture for real, every
   path touched must match the pattern. **A sound over-approximation contains
   every observed run by construction, so no oracle can ever disagree with it.**
   A claim no observation can refute is not evidence — the same principle as a
   gate that cannot fail not being a gate.

## Not done

Each decided from a measurement kept with the thing it decided.

| not done | why | numbers |
| --- | --- | --- |
| a third-party parser | swapping loses more than it gains | `reader/examples/tree-sitter-probe.rs` |
| parsing regexes | biggest by volume; a regex names no file | `reader/src/bin/opacity.rs` |
| opening scripts on disk | what `deploy.sh` held *then* is unrecoverable | — |
| `deno` and `bun` inline code | a handful of calls, and neither spells its flags as node does | `reader/src/shell_ops.rs` |
| a policy-refusal channel | the refused bucket had one member | memview#820 |
| confirming `\|\|` from a non-zero exit | reachable by arithmetic, and tiny | `reader/src/shell.rs` |

The timeline takes tool calls as well as the shell since 2026-08-17 — `Read`,
`Write`, `Edit`, `Grep`, `Agent` and the two web tools each produce a row, and
`agents::TOOLS` is the list, **taken from the corpus rather than from the tool
list anybody remembers**: `Task`, `MultiEdit` and `NotebookEdit` appear in it
zero times. It records everything that happened rather than the notable part of
it, and `doing.rs` says why.

Rows group into **episodes** — the stretch of work one instruction produced,
bracketed by user turns. The boundary is *observed*, never inferred from a gap or
a change of subject: inference can merge two instructions into one episode, and a
merge is unrecoverable downstream, where a duplicate bracket is only noise.
`doing.rs` carries the rule.
