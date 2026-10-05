# The evaluator

What a command will leave in the files it writes, computed from its text and
whatever state it is given, before it runs. `reader/src/predict/` is the code;
each module's doc-comment is the authoritative account of what it follows, and
this document carries the decisions, the rules that cut across modules, and
the findings that shaped them.

It is the third of the reader's questions, beside the two the other documents
answer: [execution-model.md](execution-model.md) says *what is this text*,
[reader.md](reader.md) *what did it touch*, and this *what will the files
hold afterwards*. [reader.md](reader.md#three-questions-three-layers) lays the
three side by side. [concept-model.md](concept-model.md) is the layer above
all of them.

## Two settings, one evaluator

Two questions per command, neither well answered before this existed:

- **prediction** — what a command will name and change, before it runs;
- **reconstruction** — what it did, from history.

Diffing the two is the point.

Prediction is a **pure function** of a command and whatever state it is given:
the files it will write, as far as the text and that state determine them, or a
refusal that names the construct it could not follow. It never opens a file and
never runs a program. What differs is only what it is given:

- **From history** it is given the text alone. The filesystem that answered the
  command has moved on, so a write whose result depends on a file's old contents
  is undetermined, and says so.
- **Before a live call** it is also given the current text of each file the
  command will write, read at the edge by the console
  ([agent-console.md](agent-console.md#what-a-command-will-change-before-it-runs)).
  The same evaluator, with more of its inputs known, can then say what each file
  will hold afterwards.

IO lives at the edges and nowhere else: reading the files before the call is one
edge, reading them after it is the other. Everything between is a value.

## Exact or refused

**What the evaluator cannot follow yields no prediction, never an
approximation.** It is refused by name and counted, the way every other gap in
the reader is, and that census is what orders the next construct to teach it.
Observation is never a substitute for it: a diff taken from the files alone
would show a change nobody here understood, and hide the gap that says what to
build.

**Every write is predicted or refused, never left out.** A prediction is a file's
text afterwards, or that it will not exist. Any program that writes files itself
and is not modelled has them named by the shell tables and refused, so no write
it knows of goes unmentioned. A program the tables do not know may have written
any file, and nothing predicted before it survives it (see *Programs it does
not know*). `predict-report` lists what the reconstruction knows is written and
the evaluator never mentions, which is what is still out of its sight.

### A prediction is exact, and a set is exact

A prediction is a file's text afterwards, its absence, or a finite set of
those: `if grep -q x f; then sed -i … f; fi` with the test undecided leaves `f`
as one of two texts, and saying so is exact — the set is the outcomes, nothing
missing and nothing extra. It is the same shape as `S ⊆ L` for paths in
[reader.md](reader.md): a described set, refuted after the call by an actual
text that is not in it. "Never an approximation" therefore forbids two things:
a single guess picked from the set, and a set padded with a text the command
could not produce.

- Built for the shell `if` and `case` and the Python `if` (2026-09-30); a
  `case` whose word is known runs the arm it selects, and one whose word is
  not runs every arm, and none where no pattern is `*`, joined alike: both arms
  run from the state before it and are joined, a file they leave differently
  holding one of their texts; appending to it appends to each, and reading it
  for its text is refused (`one of several texts`). Each file's set is exact on
  its own; which members go together across files is not kept. Arms that end
  the shell differently, or bind a name differently, leave the `if` refused or
  the name unknown; in Python, arms that raise, return or jump, or define a
  function or lambda, leave it refused. The console draws each member as its
  own diff, and the after-look agrees when the file holds any of them; a
  divergence is kept with all of them.
- A jump in a region not followed — `exit`, `return`, `break`, `continue`,
  `sys.exit()` — makes what follows run only sometimes, until the function
  or loop it leaves ends. One that fails the call (`exit 1`, `raise`) is
  excluded: the call is not checked, and the success the prediction assumes
  is the world where it was not taken.
- A member enters the set only from a branch the text has. Two undecided
  tests over one file give four texts; a loop whose count is unknown gives a
  set nobody can write down, and stays refused. Exact or refused, and a set is
  exact when it is finite and every member is reachable.
- A check can only refute a set. It cannot show that an extra member was
  reachable, so a padded set passes every check — the same blind spot as sound
  over-approximation, and the guard is construction, not the oracle.

### The prediction assumes each command succeeds

A write after `||` is only sometimes made, and is not followed. Whether the
call really went that way is for the check after it to say; only a call that
succeeded is checked (see *The live oracle*). A test is the exception: it is a
question, not a command that might fail, so it is not assumed to succeed.

## Sight

The evaluator is a pure function of the text and of what it is shown. What it
is shown is decided by these rules (the user, 2026-09-28):

- **It reads; it never runs.** A file's text, a directory's names, and nothing
  a process would have to compute. What a program does to text — `cat`, `sed`,
  `grep`, `sort`, `head` — is modelled here, in Rust, so that the safety of a
  prediction comes from this code and not from the program being harmless on
  the day it was called: `cargo fmt` changes nothing today and may install
  something tomorrow, and a `sed` that only reads is one flag away from one
  that writes. No local process is started for a prediction, ever.
- **On another machine, the same reads and only those.** A file there is read
  with `cat` and a directory listed with `ls`, over ssh, with a literal path
  quoted for the remote shell, and no other program. That list is closed. The
  end state is a program of ours on each host answering the same questions
  through a fixed protocol, so no shell re-splits anything and the remote side
  is under the same rule as this crate: it can read and has no code path that
  writes. Until it exists, the two commands are the whole remote vocabulary.
- **Sight is asked for, not handed over.** The evaluator asks for a file or a
  directory when it reaches the command that needs it, so a path that only
  becomes known part-way through (`cd "$(cat where)" && …`) can still be
  read. What was shown is recorded with the prediction, which is what makes a
  finding replayable. From history nothing is shown and every ask is `not
  read`; in a test the fixture answers; before a live call the console answers
  from disk.
- **A directory's names come in no order.** A listing (`glob.glob`,
  `os.listdir`, `Path.glob`, `iterdir`) is a set of names: `sorted` of it is
  exact, and a loop over it is refused, since the order a directory gives
  is the filesystem's and the order bash sorts a glob in is the host's
  collation, which the text does not state (an empty locale here collates
  as `en_US.UTF-8`, not bytewise). The run's own writes and removals there
  are merged in; after a directory is made or removed untracked (`mkdir`,
  `os.makedirs`) no listing is known. The console lists at most ten thousand
  names and keeps each listing with the row, as it keeps files.
- **Git's objects are files, and reading them is on the list — later.** What
  `git checkout -- f` restores is the index's blob, which sight could read; it
  means decoding git's storage, through a reviewed Rust library rather than
  through `git` itself. Not before the simpler constructs are done; until then
  a tree-rewriting git command withdraws everything, as it does now.

## Shell

Straight-line shell, in order: top-level commands run one after another, and a
write replaces or extends what the file held at that point.

### Sequence, groups and loops

A `for` over words the text spells out runs its body once per word; a brace
group is its commands; a subshell is the same with its `cd` and its bindings
kept inside. A loop or branch not followed forgets only the names it binds (all
of them after `read`, `declare` and the like). A group, a subshell or a loop
with an output redirect opens its file once and collects what each command
inside prints without a redirect of its own (only the last member of a
pipeline prints into it; a command whose output is not modelled leaves the file
refused under its name).

A script handed to another shell (`bash -c`, `nix-shell --run`) is the same
language against the same files, so it is followed in place, its `cd` kept
inside it. A command it runs is followed as the shell it amounts to.

### Redirects and heredocs

A heredoc into a file carries its whole new text; `echo`/`printf` into a file
is the same. An unquoted heredoc, to a file or to Python, is expanded as bash
expands it: a name the run knows is substituted, a `$` that starts no
expansion stays (`.*$"` in a regex), and anything else (`$(date)`, an unbound
name, `${x:-y}`) refuses it. A redirect to a path the text does not determine
withdraws what came before, as any write to an unknown path does.

### Variables and expansions

A variable is known while the text bound it to a literal and no builtin since
could have rebound it; `$HOME` and `$PWD` are known, and `${x%%pat}` and its
three siblings are computed when the pattern is spelled out.

What a word runs as it expands runs before its command: a `$( )` is followed as
a subshell whose output is the word's, so a program inside one withdraws what
came before as any unknown program does; one inside a parameter's operand runs
only sometimes, and `<( )` beside the command. A `$( )` has the value of what
it printed, its trailing newlines removed — unquoted, only where splitting and
globbing change nothing.

### Tests and branches

A test is a question, not a command that might fail, so it is not assumed to
succeed: one sight can answer (`[ -f x ]`, `[ -d x ]`, `test -z "$v"`,
`[[ a == b ]]`, `true`, `false`) steers `&&`, `||` and `if` exactly, and after
one it cannot, what follows is only sometimes run. `grep` answers whether it
selected a line, which steers the same way; one this cannot run is not assumed
to have matched. An undecided `if` or `case` leaves a finite set (*A prediction
is exact, and a set is exact*, above).

### Pipelines and the text tools

Each member of a pipeline is followed as the subshell bash gives it, reading
the pipe as its stdin, the files as they were before the pipeline, and leaving
a path two members both change unknown, since their order is not. A pipeline
carries what each member printed to the next: `echo`, `printf`, `cat` and the
text tools reimplemented in `reader/src/predict/text.rs` — `grep`, `head`,
`tail`, `cut`, `tr`, `uniq`, `basename`, `dirname`, and `sort` only under
`LC_ALL=C`, since its order is otherwise the host's collation — each held to
what the real tools print by a test whose expected texts they printed. `wc` is
refused: GNU and BSD pad it differently. Note that in a Claude Code session
`grep` is a shell function running an embedded `ugrep`, and in a `bash -c`
child it is `/usr/bin/grep`: the modelled subset agrees with both.

### Programs that write files

`rm` of a path the text names exactly removes it, and `rm -r` everything under
it; `cp` of one file to another leaves the source's text there, once sight has
shown both (a destination it cannot show may be a directory), and `mv` the
same with the source gone. `sed -i` rewrites what sight has shown where sed's
regular expressions and Rust's agree, the match made the longest by
construction (`reader/src/predict/sed.rs`): `s`, `d`, and `a\` and `i\` with
their text on the next line — the form macOS's BSD sed, the one here, reads;
its one-line `a text` fails the call, and appending after a last line without
a newline, where BSD and GNU sed differ, is refused. Each is held to what
`/usr/bin/sed -i ''` left in a file.

Any other program that writes files itself has them named by the shell tables
and refused, and a pattern it expands itself (`ktlint -F 'src/**/*.kt'`)
stands for everything under its fixed part.

### Programs it does not know

A program the tables do not know, a script run from a file, code the text does
not show (a package script, `pnpm install` and the install scripts it runs,
`nix run`), a git command that rewrites the working tree, and a write to a
path an expansion chose may each have written any file, so nothing predicted
before one survives it, and what is read after it is not known. A refusal
names the program that ran, behind any carrier: `pnpm exec biome …` is
biome's. `cargo test`, `run`, `nextest` and `bench` are unknown programs, not
tools that write nothing: they run the project's code, and a test that blesses
a golden writes it.

Most of the time such a program wrote none of the files; *A prediction on a
named condition*, below, is how that is said without guessing.

## Python

Python is followed by `reader/src/predict/python.rs`, on the program's tree,
against the same files as the shell around it: straight-line, each call assumed
to succeed, strings, paths and open files interpreted. What a write depends on
that it cannot name makes the write a refusal, never a guess. A block it does
not follow (an undecided `if`, a loop over values it cannot list, a decorated
function) has every file it writes refused and forgotten, as a shell branch is,
named for why its test or its iterable is not known (`for over call
glob.glob`), since that is what to build.

### Values

`sys.argv` holds what the shell passed the interpreter where the text gives it
(`python3 - "$d" x` is `['-', d, 'x']`); a flag or a wrapper before the program
leaves it unknown. A function the program defines is followed in its own
frame. A function or lambda defined inside another function reads that
function's names, which a call's own frame does not hold, so a call to one is
not followed.

### Loops, comprehensions and generators

A loop runs once per element over a list the program can name: written out, a
`range`, a shown file's lines, a split string, and `enumerate`, `zip` or
`sorted` of those. A list or dict comprehension runs at once and in order with
its variables its own. A generator runs only as far as the `next`, `any`,
`all`, `join` or `list` written around it reads it.

### Lists, dicts and sets

A list is held by value where Python shares it, so a change made in place (an
item, a slice, `append`, `insert`, `extend`, `del`) is followed only when
modelled, and then every other holder of the list, under any name in any
frame, is forgotten; a list passed to a call it does not follow, stored where
it cannot see, or reached by a block it does not follow is forgotten too. A
dict is followed the same way, in insertion order, through its items, its
keys, `get`, `setdefault`, `pop` and `update`, and its views are forgotten with
it. A set is followed where its order does not matter — `sorted` of it, `len`,
`in`, `|`, `&`, `-`, `add`, `discard`, `update` — and refused where it does:
Python gives it none.

### Regular expressions

`re.sub` is followed where Python's `re` and Rust's `regex` mean the same
thing, translated rather than copied, and refused by what differs where they
do not: a backreference, a pattern that can match the empty string, a `$`
before a final newline. `re.search`, `re.match`, `re.fullmatch`, `re.finditer`
and `re.findall` are followed on the same translation, a match's groups by
number and name with positions in characters as Python counts them, and
`re.sub` with a function or a module-level lambda calls it once per match.

### JSON

`json.load` of a file sight has shown is read by the evaluator's own reader
(`python/json.rs`), with Python's rules for a repeated key, and `json.dump`
writes CPython's exact text for the indent, separators, `sort_keys` and
`ensure_ascii` it was given; a float, which the evaluator does not model, is
refused.

### Subprocesses

A subprocess whose command it cannot read is an unknown program like any
other: what was predicted before it is withdrawn, or held on the condition
that it left the file alone. One it can read is followed as the shell it
amounts to.

## A prediction on a named condition

A program the tables do not know may have written any file, so what was
predicted before it is withdrawn and what is read after it is not known. Most
of the time it wrote none of them: `scripts/dev bash -c 'cd lean && lake
build'` after a Python edit to a `.lean` file. So the evaluator runs a second
time assuming each unknown program left every file alone, and a file only that
run predicts is a **conditional** prediction, drawn with the programs it
assumed: those run before its last write, which may have changed what the
write read, and those run after it (the user, 2026-09-29).

It is still exact: the text follows from the command and the assumption, and
the assumption is named, not hidden. What the assumption does not cover, and
is refused either way:

- what a program writes itself (`./gen.sh > out`), since assuming it harmless
  says nothing about its own output;
- a file the program was told of — a path in its words or its environment
  (`OUT=/tmp/rows ./test.sh`), and everything under one — and what it is told
  on stdin when that is known (`echo '[{"file":"e2e/a.ts"}]' | node fix.mjs`);
- a file the command removed before running it: clearing an output is how a
  program is asked to write it again (`rm -f tests/golden/x && X_BLESS=1
  scripts/dev cargo nextest`);
- what a checker among the program's words rewrites, as the shell tables read
  it, inside a quoted script too: `scripts/dev cargo fmt` reformats its
  directory without naming a file; after a `cd` inside the script, where it
  ran is not known and nothing is assumed.

The after-look checks a conditional prediction like any other, but a
divergence there has two suspects, the evaluator and the program assumed
harmless, so it is kept apart (`conditional.jsonl`) and is not a finding until
the evaluator is shown to be the one at fault. `predict-report --live` tallies
the outcomes, conditional apart, and for each program assumed harmless how
often it was: one that often was not is what to model next. It also predicts
each conditional divergence again under today's evaluator, as it does a
finding, so a fix shows on the rows that called for it.

## The live oracle

**Reconstruction and prediction meet on every live call.** After the call, the
console reads the same files again and compares them with the prediction. That
comparison is the third oracle, beside the fixture shims and bash's printer — and
the only one that sees real work at full scale, because it observes the call that
was going to run anyway rather than re-executing anything.

A divergence is a defect in the evaluator, kept with the command, the files the
prediction was made from and both texts, so the prediction can be made again
under a later evaluator: `predict-report --live` does, and sorts the findings
into still diverging, agreeing now, and no longer predicted. A finding kept
without its inputs cannot become a test, which the first 48 showed: the edits
behind them were Python over files too large to rebuild by hand. Nothing kept
under `.console` is golden. Beside what the evaluator asked for, the console
keeps each file the command names, as the reconstruction reads it (up to 4 MB):
a later evaluator reaches further, and replayed on only the old asks it would
find those files unread.

What is kept, and for how long:

- a **finding** is an inbox item, deleted once its test is in `reader/tests`;
- a **refusal** row (`refused.jsonl`, beside the findings, with the files it
  was shown) is pruned after thirty days, since the command is in the
  transcript and only recent ones rank;
- a **conditional** divergence (`conditional.jsonl`) is kept apart, above;
- an **outcome** (`outcomes.jsonl`) is kept for good: each file checked, agreed
  or diverged, and each prediction never checked and why. What a file held
  after its call cannot be looked at again, and without the agreements a
  prediction that was right reads the same as one nobody looked at.

The target is a prediction that never diverges; the check stays once it is
reached, so the two cannot drift apart again.

Which calls are checked: only a call that succeeded — the hook that reports a
call's end does not fire for one that failed, so a prediction that assumed
success is never held against a call that did not get that far. A shell exit
code can hide a failure behind a later command (`python3 … ; grep …`), so a
call whose interpreter printed a traceback is dropped unchecked too. A call
sent to the background is looked at twice, when it is backgrounded and when its
task ends, and agrees if either look holds the prediction: the session's next
edit, which the check never sees, can overwrite a file while the task runs.

## What orders the work

From history no file is given, so most of the refusal census is `not read`; a
live call has its files read, and the console keeps its refusals with those
files, which `predict-report --live` predicts again under the current evaluator
and ranks — a name an older evaluator wrote is not counted, since a rename
would leave it ranking a construct that no longer exists. That ranking is the
worklist, with the caution the parser's census carries
([execution-model.md](execution-model.md#a-refusal-ranking-is-not-a-work-queue)):
it says what stopped the evaluator, not what building a construct would
unlock, because everything after an unfollowed program is withdrawn under that
program's name.

The order it grew in came from the corpus. Shell first, where the tree exists:
a heredoc into a file, then `echo`/`printf` into a file, then `sed -i`. Python,
the largest share by far, followed once it had a tree of its own
([execution-model.md](execution-model.md#the-python-tree)).

## Findings, dated

Each is a live divergence or refusal that became a rule above, kept with the
figure that sized it. Read the current state off `predict-report --live`, not
off this list.

- **2026-09-27, the unknown-program rule.** Edit scripts (`sub.py`,
  `insert_kotlin.py`) ran after a heredoc rewrote the file they had just
  written; the rule that nothing predicted before an unknown program survives
  it withdrew 2,512 of 6,575 predictions from history, every one a claim the
  text could not back.
- **2026-09-29, lists by value.** An insertion by `lines[i:i] = ins` and
  trailing lines popped in a `while` were both predicted as no change; in
  history the same blind spot had backed 118 file texts.
- **2026-09-30, told paths.** The first two conditional divergences were both
  a test told its output file in an assignment. The same day: a golden cleared
  with `rm -f` before `X_BLESS=1 … cargo nextest`, three times, and `scripts/dev
  cargo fmt` after a Python edit of `sync.rs`.
- **2026-10-01, unquoted heredocs.** A memory edit by `python3 - <<EOF` was
  refused whole for a regex, and three more for a name the command had just
  bound; expanded as bash expands it since. The same day, four certain findings
  from `cargo test` and its siblings treated as tools that write nothing.
- **2026-10-02, stdin.** A program told its files on stdin
  (`echo '[{"file":"e2e/a.ts"}]' | node fix.mjs`) was assumed to have left them
  alone.
- **2026-10-04, `launchctl` and a lost `cd`.** Four findings: a log removed
  before `launchctl bootstrap` or `submit` was predicted absent, and the job
  the plist named wrote it — `launchctl` starting a job is an unknown program
  now, as `nix run` is. One more: `… && cd .. && …; cp /tmp/bak mac-mini/f.py`
  kept the `sed -i`'s text, because a `cp` whose destination is relative to a
  `cd` that only sometimes ran came back from the tables as a read of its
  source; it is a write to an unknown path, and withdraws what came before.
  Fixed 2026-10-05, with the six older findings the rules of 09-30 and 10-01
  had already closed; the inbox is empty.
