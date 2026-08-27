# The memory model

What the corpus *is*, as opposed to how the viewer serves it or how the linter
checks it. This is the document the rest of memview assumes and never states —
until now the model lived in two places, neither of them readable: the corpus's
own `feedback_*` rules, and comments inside memview's tests.

⚠ **No figures here** (the README's rule, and the corpus's own —
`feedback_a_count_in_prose_rots`). Sizes, counts and rates move; read them off
`memory-rank` and `memory-lint`. What is written down here is shape.

## The root and the traversal

`MEMORY.md` is **the root**, and the root is not a table of contents. It is
injected into every session, every turn, before anything is asked. Everything
else in the corpus is reached by following a link out of it.

The analogy that fits, and the one to keep: **a root entry is what is thought
immediately; everything else is what is thought after some traversal.** A person
does not consult a file to remember that a hot pan burns — that arrives with the
situation. Working out which of two libraries handled a bug last spring is a
different act, and it is fine for it to be one.

So the corpus has exactly two tiers, and they are not "important" and
"unimportant":

    root        present without being asked for. Costs every turn of every
                session, forever, whether or not it is relevant.

    traversed   present when reached. Costs nothing until opened, and the cost
                is paid by the session that wanted it.

**The tiers differ in WHEN, not in worth.** A memory can be profoundly important
and belong in the traversed tier, because by the time it matters you will be
looking for it. A memory can be minor and belong at the root, because the moment
it matters you will not know to look.

## Why the root is scarce, and scarce in a way that bites

The injection has a hard size ceiling. Past it, **the root is truncated and
nothing tells the reader which part went missing** — the session simply thinks
with a partial root and has no way to notice. That is the failure this tier is
uniquely exposed to: a body that is too long is a slow read, but a root that is
too long is *silently incomplete*.

⚠ Check the current size against the ceiling before adding a line. If the root
is over, adding a good line evicts an unknown other line rather than joining it.
That is not a trade anyone chose, and it is invisible in the diff.

This is why the root's economy is not "is this true and useful" — everything in
the corpus should be true and useful. It is: **does this need to be thought
before the situation names it?**

## The two shapes a root line can take

A root line is a hook, never content (`feedback_index_not_content`). Bodies live
in files and are opened on demand — anything injected every turn that carries its
own substance is spending the scarcest space on the least-needed words.

Given that, a line is one of two things, and which one is a decision to take
*before* writing it (`feedback_pointer_or_tripwire`):

  * **A pointer** names a subject so you can find the file when you go looking.
    It works by being findable. Its success looks like a low open rate followed
    by an open at the right moment.

  * **A tripwire fires from the root itself.** It exists because by the time you
    would have gone looking, the damage is done — the wrong command has run, the
    plausible-but-false belief is already load-bearing. It must state its claim
    in the line, because the line is the whole intervention.

⚠ **A low open rate means opposite things for the two.** For a pointer it may
mean the subject is quiet. For a tripwire it may mean the hook is WORKING — the
reader was warned and never needed the file. Ranking cannot tell these apart from
opens alone, which is the standing hazard in any automated demotion.

## The invariant is reachability, not membership

`MEMORY.md` is not required to name everything. What is required is that every
memory is reachable: listed at the root, or `[[linked]]` from something that is
(`feedback_memory_index_is_the_working_set`).

Two consequences worth stating plainly:

  * **Demoting a memory is not deleting it.** A file dropped from the root but
    linked from a live hub is one hop further away, not gone. Some documents are
    deliberately one hop deeper and should not be "restored".
  * **Demoting an unlinked memory strands it.** Give it a home first. `memory-rank`
    separates these two cases for exactly this reason, and the distinction is the
    whole safety of the operation.

## Two populations belong at the root, and they earn it differently

Pippijn's framing, 2026-08-27: the root is **direct memory**. Two kinds of thing
belong in it, and conflating them is why cuts keep going wrong.

    RECENT      what is being worked on now. Belongs at the root because it is
                live, not because it is proven. A session writes these, and it
                writes them constantly — that is not a defect to suppress.

    CONSOLIDATED  what has repeatedly helped, across time and across subjects.
                Belongs at the root because the moment it matters you will not
                know to look for it.

⚠ **A single session cannot judge either one correctly, and it is worst at the
second.** A session is focused on one topic. Everything outside that topic looks
inert to it, so asked to make room it will evict precisely the consolidated
entries — the ones whose value is that they fire in situations *other than the
current one*. The judgement has to be made from evidence no single session holds.

**Consolidation needs volume AND breadth, and breadth is the half that gets
forgotten.** How often a memory is consulted is real evidence and must count —
something read constantly is doing work. But volume alone cannot separate the two
populations: forty reads by one session on one afternoon is a topic being worked,
not a rule that has consolidated. Spread across many sessions and many subjects
is what says "this fires in situations other than the one that wrote it", which
is the qualification for a root slot. That is also how consolidation works in the
analogy we are borrowing — repeated retrieval in *varied* contexts, not
repetition in one.

⚠ **So neither factor is sufficient alone, and they fail in opposite
directions.** Volume alone promotes whatever is being worked on this week.
Breadth alone punishes deep focus — a month spent on one subject produces
memories read by one session that are nonetheless exactly the working set.

So the two tiers want two different rules, and only one of them is a cut:

  * **Recent entries hold a lease.** New, used, at the root — and expiring by
    default. If a session keeps writing entries, let it; the tier is supposed to
    turn over. What must not happen is a lease quietly becoming tenure because
    nobody looked.
  * **Consolidated entries hold tenure, earned by being consulted often AND
    widely** — distinct sessions, distinct projects, distinct days, over a long
    interval. Volume says it is doing work; breadth says the work is not one
    topic's.
  * **Evict from the lease tier by age. Never evict from the tenure tier on one
    session's opinion** — that is the failure mode above, and it is the one that
    stranded 24 memories on 2026-08-07.

## What memview can and cannot measure

**Opens of `MEMORY.md` itself carry no information.** It is injected, so every
session "opened" it and the number says the same thing about everyone. memview
knows this — `tests/agents.rs::the_index_is_not_a_memory_anyone_knows` exists to
hold the line — and ranking therefore counts opens of a root line's *target*, not
of the root.

`memory-rank` counts **days, not opens** — one afternoon of forty reads is one
day of being live, the same as a quiet one. That correction is what changed the
answer when the weighting was first measured; the choice of decay curve did not.
⚠ Do not reach for the half-life to explain a ranking. A memory that looks quiet
under one curve looks quiet under all of them, because the underlying fact is
usually that it was live on about one day, ever.

Three blind spots are documented in the tool itself and bound anything built on
top of it:

  * **Unprovable shell reads are discarded.** Shell reads themselves ARE
    counted — `d39d227` gave the shell site its own arm on 2026-08-14 and reads
    rose 137% — and this document said otherwise until 2026-08-27, having
    inherited the claim from `memory-rank`'s docstring after it had already been
    fixed in code. What is still dropped is `maybe_reads`: a read whose success
    cannot be established, which the ranking collects and never consults
    (memview#1214). So the figures remain a floor, and the floor still leans
    toward whoever reads in bulk — for a narrower reason than was written here.
  * **The teaser paradox.** For the entries that work best the index line IS the
    memory — a reader acts on "no CoA" and never opens the file. Opens therefore
    under-measure the best-compressed behavioural rules, which is why `feedback`
    is reported apart from `reference` and `project` and never ranked against
    them.
  * **The ratchet.** Being listed causes opens; demoting cuts opens, which then
    justifies staying demoted. The DEMOTED BUT STILL CONSULTED section exists as
    the counter-evidence.

That leaves an asymmetry in what is *built*, though not in what is knowable:

  * **Demotion has mechanics.** Least-consulted, already-at-home, with the
    no-home cases held back — `memory-rank` proposes a set and says what it
    saves, and checks reachability of the set as a whole rather than per entry.
  * **Admission has none.** Nothing proposes that a line should JOIN the root, so
    the root grows by judgement and shrinks by measurement — a ratchet pointing
    the wrong way, and the reason it drifts over its ceiling rather than settling
    under it.

⚠ **But admission is not unmeasurable, which is the correction worth making.**
The data model already carries reads and writes *per project directory*, per
named agent — so breadth (how many distinct sessions and subjects consulted a
memory) is derivable from what is already mined. Breadth is the missing factor,
and it is exactly the one a single session cannot observe about itself.

What remains genuinely unmeasurable is narrower than "admission": it is
**whether a memory helped**. A tripwire's value is the incident it prevented, and
prevented incidents leave no trace in a record of what was read. Breadth is a
proxy for it — consulted in many contexts is evidence of general use — and it
should be treated as a proxy, not mistaken for the thing.

## Where authority lives

This document is the model. The **practice** is governed by the corpus's own
rules, which are memories like any other and win where they disagree with
anything written here:

    feedback_index_not_content              an injected line is an index, never content
    feedback_memory_index_is_the_working_set  reachability, and why not to compact on a size hook
    feedback_pointer_or_tripwire            decide the shape before writing the line
    feedback_open_full_memory_file          teasers only — open the file before citing it
    reference_memory_index_demotion_method  how a demotion is actually carried out

If this document and one of those disagree, the memory is right and this file
needs correcting — the corpus is the system of record for how the corpus is
written, and a doc in a tool's repository is not.
