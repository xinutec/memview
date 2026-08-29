# The memory model

What the corpus *is*, as opposed to how the viewer serves it or how the linter
checks it. This is the document the rest of memview assumes and never states.

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

The injection has a size ceiling. Past it, **the root is truncated and nothing
tells the reader which part went missing** — the session simply thinks with a
partial root and has no way to notice. That is the failure this tier is uniquely
exposed to: a body that is too long is a slow read, but a root that is too long
is *silently incomplete*.

⚠ **Where that ceiling actually is has never been measured, and the number in
the tool is a guess made deliberately low.** `memory-tiers` uses 24,400 bytes,
read off Claude Code's own warning text; 24.4 KB is ambiguous between 24,400 and
24,985 depending on which kilobyte is meant, and guessing high costs a silent
truncation while guessing low costs a few hundred bytes of headroom. The corpus's
own header records the observed bracket rather than the warning line: a root of
about 25 KB arrived whole, and one of 27,382 bytes lost its last forty entries.
So the cliff is real, its edge sits somewhere between those two, and **the number
this model is administered by is a warning line, not the edge.** Do not quote it
as the ceiling.

⚠ Check the current size against the ceiling before adding a line. If the root
is over, adding a good line evicts an unknown other line rather than joining it.
That is not a trade anyone chose, and it is invisible in the diff.

## ⚠ The state this model does not yet describe: over the line with no legal move

The trade below assumes a trade exists. It can fail to: the root can sit above
the ceiling while nothing is demotable and everything that qualifies is blocked
for want of room, so `memory-tiers` proposes `0 in, 0 out, net 0` and the file
stays over. That is not a bug in the tool and it is not a stalemate to wait out —
it is the model reporting that the only remaining moves are ones a rule was
written to forbid, and it is where the sustainability question actually lives.

Three things are true at once in that state and all three should be said before
anyone reaches for a cut:

  * **Demotion is blocked by ROLE, not by evidence.** Most of what ranks lowest
    is a tripwire, and a tripwire's low open rate is what success looks like.
  * **Admission is blocked by ROOM, not by evidence.** Entries have earned a slot
    and there is nowhere to put them.
  * **A held entry is not a rejected one.** #884's freeze holds part of the
    corpus out of the trade until its harvest, by design.

The honest responses are to shorten existing lines rather than remove them, to
let leases expire once the freeze lifts, or to accept the root as it is and say
so. What must NOT happen is a cut chosen because the tool printed zero.

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
  * **Demoting an unlinked memory strands it.** Give it a home first.
    `memory-rank` separates these two cases for this reason, and the distinction
    is the whole safety of the operation.

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
memories read by one session that are nonetheless the working set.

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
    stranded memories on 2026-08-07.

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
    counted — `d39d227` gave the shell site its own arm on 2026-08-14. What is
    dropped is `maybe_reads`: a read whose success cannot be established, which
    the ranking collects and never consults (memview#1214). So the figures
    remain a floor, and the floor leans toward whoever reads in bulk.
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
  * **Admission has mechanics too, and they are the same operation.** The root
    used to grow by judgement and shrink by measurement — a ratchet pointing the
    wrong way, and the reason it drifted over its ceiling rather than settling
    under it. `memory-tiers` closes it by proposing a TRADE: what has earned a
    slot, what has finished with one, and whether the root is smaller
    afterwards. Deciding the halves separately is what breaches the ceiling.

    Two details of that proposal are load-bearing. It lists **every** qualifier
    for admission, not only those the budget covers — "eleven have earned a slot
    and none fit" is a different finding from "nothing has earned one", and only
    the first argues for a demotion pass. And the arrival price is stated for
    people as well, in `MEMORY.md`'s own header: live work belongs at the root
    and writing a line for it is right, but the line is a lease, and at the
    ceiling a new one is paid for by demoting a finished one in the same edit.

    ⚠ **The bar for admission is not "does a parent already link it"** — a
    memory about current work usually does, from its own project hub, and that
    test would exclude exactly the recent thinking the tier exists to hold.
    Parentage decides whether a demotion is SAFE, never whether a line is
    warranted.

⚠ **Distance was the OTHER missing factor, and it is measured now (2026-08-29).**
Everything above describes how much a memory is used. Nothing described how far
away it is — `reachable_without` ran a walk from the index and returned a set,
discarding the depth it had to compute on the way, so reachability was a boolean
and "reached by fifteen agents from four hops out" read the same as "from one".
`store::depths_without` keeps the number and `memory-tiers` prints it beside
breadth.

⚠ **It is a second question, not a tie-breaker on breadth**, and it is printed
rather than scored because nothing has measured which way it should weigh. High
breadth at ONE hop says the traversal is already short and a root line buys
little; the same breadth from four hops out is a reader going a long way,
repeatedly, for something the root does not carry. The first reading it produced
on the live corpus was that nearly every qualified admission sits two hops out —
one step past a hub — which is a weaker case for spending root bytes than the
breadth alone suggested.

⚠ **The demotion side of the same question is NOT built.** Dropping a line moves
a memory further away rather than deleting it, so the cost of a demotion is how
far its target falls — one hop is cheap, unreachable is a stranding, and `homes`
currently reports only which of the two it is. `depths_without` takes the
demotion set for exactly this and nothing passes one yet.

⚠ **Breadth was the missing factor and is no longer missing.** How many distinct
sessions and subjects consulted a memory is derivable from what the mine already
carries — reads and writes per project directory, per named agent — and
`memory-tiers` now ranks admissions by it. It is the factor a single session
cannot observe about itself, which is the whole reason the judgement could not
be left to whoever happens to be editing.

What remains unmeasurable is narrower than "admission": it is **whether a memory
helped**. A tripwire's value is the incident it prevented, and prevented
incidents leave no trace in a record of what was read. Breadth is a proxy for
it — consulted in many contexts is evidence of general use — and it should be
treated as a proxy, not mistaken for the thing.

## Where authority lives

This document is the model. The **practice** is governed by the corpus's own
rules, which are memories like any other and win where they disagree with
anything written here:

    feedback_index_not_content              an injected line is an index, never content
    feedback_memory_index_is_the_working_set  reachability, and why not to compact on a size hook
    feedback_pointer_or_tripwire            decide the shape before writing the line
    feedback_open_full_memory_file          teasers only — open the file before citing it
    reference_memory_index_demotion_method  how a demotion is carried out

If this document and one of those disagree, the memory is right and this file
needs correcting — the corpus is the system of record for how the corpus is
written, and a doc in a tool's repository is not.
