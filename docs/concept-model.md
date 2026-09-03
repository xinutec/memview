# Concept model

Design for the layers above the reader: **lifting** what the fleet executed into
the concepts it was executing, and **lowering** a concept back into a command
that does the same thing.

**Status: design. Nothing below is built.** Where a step needs sizing, the
instrument that would size it is named — a count written here would rot.
[execution-model.md](execution-model.md) governs the syntax underneath and
[reader.md](reader.md) the effects; both stand unchanged, and every decision
recorded there binds this layer too — soundness refused, outputs never
concretising subjects (memview#1078), no policy-refusal channel (memview#820).

## Purpose

[execution-model.md](execution-model.md) answers *what is this text*;
[reader.md](reader.md) answers *what did it touch*. Neither answers **what was
it for** — and that is the first thing a person says about any command.
`sed -n '1,40p' file` is a read of one path at L3 and *paging through a file*
to anybody watching; `until curl -s host; do sleep 5; done` is a loop over an
undetermined subject, and *waiting for a service to come up*.

The ambition, stated as a law rather than a wish: a **bidirectional,
semantically lossless transformation** between executable text and conceptual
structure. Lossless semantically, not syntactically — quoting, layout, and
ultimately *the choice of language* normalise away, and everything that decided
what happened to the world is carried.

## Decided — 2026-09-03, with Pippijn

Four questions this design left open were answered before any of it was built:

- **The ask card is the first consumer.** Gates first, because the lens does
  not exist without them — but a verification-only loop is a gate with no
  product behind it. Approval today reads argv, which means approval reads
  *spelling*, and every payload trap in the corpus is a spelling that hides the
  act. The card must also render the honest miss — "no concept, here is the
  L2 reading" — which forces the no-absorption rule from the first screen.
- **Cross-language from day one, because it is the cheaper option.** The merge
  is already done one level down: `program.rs` is the shared answer-type and
  L3 reads `sed -i`, `perl -pi` and `write_text` into the same effect. A
  bash-only `Rewrite` would be a concept shaped like one spelling, plus a
  migration. The `Edit`-tool arm is deferred — not hard, but trivially a
  concept already, so it proves nothing about the lens; it joins when the ask
  card needs uniformity.
- **The vocabulary is mined; the automation roadmap is the null hypothesis,
  never the seed.** Designed vocabularies lose to counted ones everywhere in
  this repository's record. The roadmap's use comes *after* the census: diff
  its target list against the mined ranking, and the disagreement is the
  finding — a roadmap item the corpus barely holds, or a top idiom no roadmap
  entry names.
- **Episode→script mining stays out of this round, and constrains it anyway.**
  Generating scripts from an ungated concept layer is the hand-waving the
  Ceiling section refuses. But recurrence detection later requires concept
  equality **up to holes** now — two `Rewrite`s of one file on different days
  must compare equal — so that `PartialEq` is a first-lens decision, not a
  second-programme feature.

## The tower

| | object | lift | lower |
| --- | --- | --- | --- |
| L0 | text — the corpus row | — | — |
| L1 | tree — `reader/src/syntax/` | parse | print, under the round-trip law |
| L2 | operations — `shell_ops::Op`, `program.rs` | classify | **none** |
| L3 | subjects — `shell_files`, `S ⊆ L` | resolve | **none** |
| L4 | concepts — **this design** | lift | lower |
| L5 | activity, episodes — `activity.rs`, `doing.rs` | classify | none, by design |

L1 already has both directions and a law; everything above it is one-way today.
`activity.rs` says so about itself: *"deliberately lossy: an `Activity` cannot
be turned back into the command it came from, and is not meant to be."* That is
the line this design draws differently — between a **classification** (names a
kind) and a **representation** (carries enough to regenerate). L5 stays a
classification; L4 is the representation that was missing between them.

**The generalised round-trip law.** For the pair (lift, lower) at any level:

```
lift ∘ lower = id        lowering a concept and lifting the result is identity
lower ∘ lift ≠ id        permitted: the text normalises — layout, quoting,
                         and at L4 the SPELLING of the concept
```

The same shape as L1's law, in the vocabulary the bidirectional-transformation
literature calls a lens (get/put). And the same constraint carries up:
**"the tree is sufficient" becomes "the concept is sufficient"** — lowering
takes no text from the level below. A concept that can only be printed by
consulting the command it came from is not a concept yet; it is an annotation.

⚠ **The law cannot see a systematic mislift, and that is already known at L1.**
A wrong concept that lowers to text lifting back to the same wrong concept
satisfies the law perfectly. The gates generalise with it:

- **gate 1** — the law itself: `lift(lower(c)) = c` over every lifted corpus
  command.
- **gate 2** — the level below is the authority: `lower(lift(t))` must read at
  L2/L3 **identically** to `t` — same operations, same subjects, same holes.
  The reader is to L4 what bash's own printer is to L1: an independent judge of
  the lowered form, and it already exists.
- **gate 3** — is the lowered text valid at all: `bash -n`, unchanged, plus the
  PATH-shim oracle on fixtures (`reader/tests/oracle.rs`) for semantics.
- **gate 4** — the author's own statement, below: a lift that contradicts the
  recorded description is a finding about one of them.

## Language choice is spelling

At L1, `'a'` = `"a"` = `a` — one tree, quoting derived at print time. At L4:

```
sed -i 's/a/b/' f
perl -pi -e 's/a/b/' f
python3 - <<PY … Path('f').write_text(re.sub('a','b', …)) … PY
Edit tool: {file_path: f, old_string: a, new_string: b}
```

one concept — `Rewrite { subject: f, substitution: s/a/b/ }` — four spellings.
The reader already reads all four into the same L3 effect (the `perl -pi`
correction alone moved thousands of uses from read to write; reader.md records
it), so the equivalence is not speculative: it is what the effect layer
already asserts. The concept's printer picks the canonical spelling the way
L1's printer picks canonical quoting, and the original language survives as
provenance, not as structure.

⚠ **This is where the tool calls join.** Two thirds of the fleet's file changes
never pass through `Write`/`Edit` (the measurement that started the reader);
the whole apparatus since has been making shell edits as visible as tool edits.
At L4 the distinction inverts and disappears: an `Edit` call **is** a concept
stated directly — the tool input is already typed, already parametrised,
already invertible. Lifting Bash meets the tool calls at the level where they
were always the same thing, and `doing.rs` — which already merged them into one
timeline — is the precedent.

## What a concept is

Typed, parametrised, invertible. The parameters take the value domain the
reader already has — exact, located language (`S ⊆ L` with a locus), or a
hole — so a concept inherits precision instead of flattening it:

```
Rewrite  { subject, substitution }          sed -i / perl -pi / python / Edit
Page     { subject, range }                 sed -n '1,40p' — the corpus's
                                            commonest use of sed by far
                                            (shell-files prints the tally)
Poll     { probe, until, every, bound }     until …; do sleep …; done
Glance   { repo }                           git log --oneline -N && git status
Probe    { question, subjects }             the compound inspect-several-things
                                            command with echo "---" separators
```

The vocabulary is **mined, not designed** — the census below ranks what the
corpus actually holds, and the seeds above are shapes reader.md already tallies
as idioms. A concept is admitted the way a construct was at L1: biggest first,
refused by name until built.

⚠ **A catch-all concept is the absorbing parser, one level up.** `Run { argv }`
covering everything unrecognised would take the lift rate to 100% on day one
and mean nothing — the L4 spelling of "coverage starts low on purpose". A
command with no concept stays an L2/L3 leaf, embedded in the concept stream the
way an unknown verb stays counted: **named / described / counted**, the
three-part artefact at every level.

⚠ **A lift rate is not understanding.** One `Poll` concept can swallow
thousands of commands while its `probe` parameter — the half a person would ask
about — goes unread. The depth measure from
[execution-model.md](execution-model.md) applies unchanged: report per command,
per byte, per node, and share descended.

**Recognition is a separate pass, exactly as Embedding is.** Concepts attach to
a pure L2 reading (`Concept { kind, params, span }`); improving a recognition
rule changes no tree and no operation. The reason is the one already written:
a recogniser folded into the layer below makes every improvement above reshape
the record beneath it.

## The corpus is self-labelling at this level too

Every `Bash` tool call carries a `description` — a stated intent beside the
command, written at generation time by an author who did not know a lifter
would ever read it. Sized 2026-09-03 over six transcripts: 201,466 Bash calls,
**97.6% carrying a description**. The corpus row (`{at, cmd, cwd, ran}`) does
not carry it; mining it is the first instrument below.

That is a parallel corpus of (command, stated intent) pairs, and it is to L4
what `declare -f` is to L1: an independent second reading of the same text,
produced by something that is not this code. Two uses, in order of trust:

- **calibration** — cluster descriptions over lifted concepts; a concept whose
  descriptions scatter is mis-cut, and vocabulary the descriptions use that no
  concept covers is the queue, ranked.
- **adjudication** — a lift that contradicts its description is a finding.
  ⚠ **About one of them, not automatically the lift**: the description is a
  claim by the same author, and the fleet already knows intent and outcome
  drift (the console reports intent, not outcome — the agent-console docs carry
  that lesson). Where they disagree, the effects at L2/L3 are the tie-breaker;
  a description the effects refute is itself worth surfacing, at scale, as
  *what the fleet says it does versus what it does*.

## Holes are carried, never filled

The reader's floor is `program::Why::Outside` — a value that never was in the
text, counted by the permanent census (`python-report --why`). Concepts inherit
those holes as **parameters**: lifting `Rewrite { subject: hole }` then
lowering yields a command over the same hole, and the losslessness claim is
exactly that — *the same holes*, not fewer.

Two standing decisions bound this from both sides:

- **memview#1078, decided against:** transcript output does not concretise an
  abstract subject; that half is inference and the reader refuses to guess.
  Concepts do not reopen it.
- **No ⊤** (reader.md, *Why this is not sound abstract interpretation*): a
  concept over an unknowable parameter carries the hole; it does not widen to
  "rewrites anything".

⚠ **Concept parameters hold verbatim fragments — a substitution program, a
probe — and the artefact boundary already has a place for that.** `doing.rs`'s
rule is derived-never-verbatim, and it stays; `effects.json` is where verbatim
text lives. Concept artefacts sit on the effects side of that line, never in
`agents.json`.

## What it buys

Ordered by how directly the evidence supports it:

1. **Prediction a person can approve — the declared first consumer, above.**
   Past and future are the same object with a different `t` (reader.md). Lift
   a command *before* it runs and the ask card can say what it is — the
   concept — and what it will touch — the predicted subjects. Approval today
   reads argv; a lifted ask reads *"Rewrite src/geo/velocity.ts in place"*
   with the L3 prediction beside it.
2. **Search in concept space.** *Every poll loop in fleet history; every
   in-place rewrite of gate.json; every glance at this repo* — questions the
   flat index cannot ask and the concept stream answers by construction.
3. **Diff said against did, at scale.** Gate 4 run over history: where
   descriptions and lifts disagree, ranked. Free once both exist.
4. **Automation mining.** A concept sequence that recurs across episodes is a
   candidate program; lowering it with its holes as parameters is a script
   drafted from history. That queue feeds the fleet's standing programme of
   moving work off ad-hoc shell onto committed, typed tools — with counts
   instead of intuition deciding what is worth committing.

## Ceiling

How ambitious can this be? The honest ceiling, top down:

- **The whole transcript as one typed stream.** Bash calls, tool calls, and
  carried programs lift into one concept vocabulary; sessions become programs
  in it, with observed episode boundaries as block structure. L5's episodes
  stop being rows about work and start being *bodies of it*.
- **Episodes lower to parametrised scripts.** The generalised law makes this
  checkable rather than generative hand-waving: a lowered episode must lift
  back to the same concepts, read at L2/L3 to the same effects, and pass the
  shim oracle on fixtures.
- **The floor is named and does not move.** `Why::Outside` at L3, the
  no-concept remainder at L4, and holes carried through every lowering. A
  claim of losslessness is falsifiable at every level because each level has
  an independent judge — which is the property the whole tower is built to
  keep, and the reason "semantically lossless" can be a law here rather than
  a slogan.

## Method

Unchanged from the two layers below, restated once:

1. **Census first.** Rank what the corpus holds; the seeds above are hunches
   until the instrument prints them.
2. **Refuse by name.** The ranked refusals are the queue; the tail is counted.
3. **Ablate.** Undo the rule; the test must fail.
4. **Make the census permanent from the first build.** #1142 rebuilt three
   temporary inventories before keying misses by reason; the concept layer
   starts with its `Why` field, not with probes.
5. **No invented test input.** The corpus is the suite; fixtures exist for the
   semantics oracle only.

## First three instruments

1. **Description mining.** Carry `description` into a corpus artefact (its own
   file beside `bash-corpus.jsonl` — the row shape there is load-bearing for
   the #1130 merge and does not change casually). Report: presence over time,
   vocabulary, length; cluster against `Activity` as the zeroth lift-check —
   L5 already names kinds, and descriptions that disagree with `activity.rs`
   are the first calibration finding either way.
2. **Concept census.** Over `shell_ops::Op` sequences per command, rank the
   recurring shapes; print the candidate vocabulary with counts and the
   remainder. This is `shell-report` one level up, and like it, the report is
   the method rather than a status line.
3. **The first lens.** Take the census's top concept; build lift, lower, and
   all four gates over it; ablate. One concept end-to-end proves the tower's
   plumbing the way round 1's 42.5% proved the grammar's — the rate is
   irrelevant, the law holding is the point. Two acceptance tests come from
   the decisions above: the concept must lift from at least two languages'
   spellings and compare equal, and two occurrences differing only in their
   holes must compare equal — the equality recurrence detection will later
   stand on.
