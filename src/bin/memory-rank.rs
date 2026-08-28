//! Which memories the index is still earning its size on, from history.
//!
//!     cargo run --release --bin memory-rank [-- --half-life 7]
//!
//! `MEMORY.md` is loaded into every session, so its cost is paid constantly
//! while its value is only in what is live *now*. Which entries those are has
//! been a guess made under size pressure — and on 2026-08-07 a guess dropped 24
//! entries without linking them first, stranding 24 memories that still existed
//! and could no longer be recalled. This is the evidence that guess was missing.
//!
//! ⚠ **A REPORT, and deliberately not an editor.** What is live is a judgement:
//! a memory can be correct, rarely opened, and exactly the thing that must be in
//! front of somebody every session. This ranks and marks; the cut stays
//! Pippijn's. See `feedback_memory_index_is_the_working_set`.
//!
//! ## What is counted, and what that misses
//!
//! **Days, not opens.** One afternoon of forty reads is one day of being live,
//! the same as a quiet one — the correction that changed the answer when this
//! weighting was first measured, where the choice of curve did not.
//!
//! **Only `Read` and `Edit` tool calls.** Mentions are unusable: context
//! re-injection made one memory's name recur 3,370 times in a single transcript,
//! and the index itself is injected every session, so every name in it appears
//! constantly whether or not anybody looked.
//!
//! ⚠ **A shell read IS counted — and this said the opposite for two weeks.**
//! `d39d227` (2026-08-14) gave the shell site its own `memory_of` arm and reads
//! went 2,646 → 6,259, +137%. The claim that used to stand here outlived it and
//! was believed: #1214 was filed on a wrong premise taken from this very
//! paragraph, which is what a stale docstring costs.
//!
//! ⚠ **What IS discarded is `maybe_reads`.** A shell read whose success cannot
//! be established — after `&&`, or inside a script with one exit status — is
//! collected under that name and never consulted by the ranking, so a memory
//! whose only evidence is unprovable access still reads as never opened
//! (#1214).
//!
//! ## The two hazards it prints rather than hides
//!
//! **The teaser paradox.** For the entries that work best the index LINE is the
//! memory — a reader follows "no CoA" or "terse" straight from the teaser and
//! never opens the file. So opens under-measure exactly the rules doing their
//! job most efficiently, and a naive frequency cut would demote the
//! best-compressed behavioural rules first. `feedback` is therefore reported
//! apart from `reference` and `project`, never ranked against them.
//!
//! **The ratchet.** Being listed causes opens; demoting cuts opens, which then
//! justifies staying demoted. The measurement is entangled with the intervention
//! and drifts one way. The DEMOTED BUT STILL CONSULTED section is the counter-
//! evidence: anything there was reached without the index carrying it.

use std::collections::BTreeSet;

use anyhow::{Context, Result};
use memview::agents::{Agents, HALF_LIFE_DAYS, day_number, weighted};
use memview::store::{Corpus, homes_for, index_entry_cost, index_links, reachable_without};

/// How a memory stands: what it cost, what it was used for, and whether the
/// index is what is holding it up.
struct Standing {
    name: String,
    /// Weighted days it was opened, at the trusted half-life and at half of it.
    read: f64,
    read_halved: f64,
    /// Weighted days it was changed. Kept apart from opens on purpose:
    /// consulting a memory and maintaining it are different claims on the index,
    /// and a memory nobody reads but somebody keeps correct is an archive entry
    /// in good standing rather than a candidate.
    edit: f64,
    /// Opens the miner could not prove happened — a shell read after `&&`, or
    /// inside a script with one exit status.
    ///
    /// ⚠ **Shown, never scored.** Counting these as opens overstates the record,
    /// which is why `MemoryUse` keeps them apart; scoring them at a discount
    /// would invent a factor, which `docs/memory.md` warns against. This list is
    /// advisory, so the honest move is to put the evidence in front of whoever
    /// decides. Measured 2026-08-27: 6 memories corpus-wide have no proven open
    /// and some unproven one, 2 of them indexed (#1214).
    maybe_reads: usize,
    /// Days since it was last opened at all, or `None` if never.
    last_open: Option<i64>,
    /// Whether `MEMORY.md` links it directly.
    indexed: bool,
    /// Bytes its index line spends, which is what a demotion actually recovers.
    entry_cost: usize,
    /// Reachable memories that already link it — the homes a demotion could land
    /// in without stranding it.
    homes: Vec<String>,
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let half_life = args
        .iter()
        .position(|a| a == "--half-life")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.parse().ok())
        .unwrap_or(HALF_LIFE_DAYS);

    let home = std::env::var("HOME").unwrap_or_default();
    let memory_dir = std::env::var("MEMORY_DIR")
        .unwrap_or_else(|_| format!("{home}/.claude/projects/-Users-pippijn-Code/memory"));
    let artefact =
        std::env::var("AGENTS_FILE").unwrap_or_else(|_| format!("{home}/.claude/agents.json"));

    let corpus = Corpus::load(&memory_dir)?;
    let mined = Agents::load(std::path::Path::new(&artefact)).with_context(|| {
        format!("reading {artefact} — mine it with: cargo run --release --bin agents")
    })?;

    // Beside the roster rather than inside it: `/api/agents` must not carry
    // this, so the miner writes it to its own file. See `agents::Agents`.
    let days_file = std::path::Path::new(&artefact).with_file_name("memory-days.json");
    let memory_days: std::collections::BTreeMap<String, memview::agents::MemoryDays> =
        serde_json::from_str(&std::fs::read_to_string(&days_file).with_context(|| {
            format!(
                "reading {} — mine it with: cargo run --release --bin agents",
                days_file.display()
            )
        })?)?;

    // ⚠ **Refuse rather than rank on a mine that has not seen the corpus.**
    // Every figure below is anchored to `generated`, so a stale artefact does
    // not merely omit recent memories — it moves the day that every age is
    // measured from, silently. On 2026-08-27 that produced breadth figures for
    // memories written after the mine and very nearly a demotion argument built
    // on them; the artefact had carried a `generated` field the whole time and
    // nothing made a reader look at it (#1210).
    //
    // The memory directory is the input that matters here: a memory written
    // after the mine is one this cannot have ranked. Transcripts are excluded
    // deliberately — they change constantly and would make this refuse always,
    // which trains people to pass the override. See `agents::freshness`.
    let projects =
        std::env::var("PROJECTS_DIR").unwrap_or_else(|_| format!("{home}/.claude/projects"));
    let freshness = mined.freshness(
        &[std::path::Path::new(&projects)],
        std::env::var("CLAUDE_CODE_SESSION_ID").ok().as_deref(),
        &home,
    );
    if freshness.is_stale() && !args.iter().any(|a| a == "--stale-ok") {
        eprintln!(
            "agents.json was mined {} and {} memories were written since:",
            freshness.generated,
            freshness.unseen.len()
        );
        for path in freshness.unseen.iter().take(5) {
            eprintln!("    {path}");
        }
        eprintln!(
            "\nranking them would measure days from the mine, not from now.\n\
             re-mine:  cargo run --release --bin agents\n\
             or pass --stale-ok to rank anyway, knowing the figures lean old."
        );
        std::process::exit(2);
    }

    // The artefact's own stamp, not the wall clock, so the report is a property
    // of the mine and re-reading it never changes what it says.
    let today = day_number(&mined.generated).unwrap_or(0);
    let index = corpus.index_md.clone().unwrap_or_default();
    let listed: BTreeSet<String> = index_links(&index).into_iter().collect();

    // Everything the index reaches by link, which is the invariant a demotion
    // must not break. Recomputed per candidate below, without its own line.
    let reached = reachable_without(&corpus.docs, &index, &BTreeSet::new());

    let mut standings: Vec<Standing> = corpus
        .docs
        .keys()
        .map(|name| {
            let days = memory_days.get(name);
            let maybe_reads = mined
                .agents
                .iter()
                .filter_map(|agent| agent.memories.get(name))
                .map(|use_| use_.maybe_reads)
                .sum();
            let reads = days.map(|d| d.reads.clone()).unwrap_or_default();
            let edits = days.map(|d| d.edits.clone()).unwrap_or_default();
            Standing {
                read: weighted(reads.iter().copied(), today, half_life),
                read_halved: weighted(reads.iter().copied(), today, half_life / 2.0),
                edit: weighted(edits.iter().copied(), today, half_life),
                maybe_reads,
                last_open: reads.iter().max().map(|d| today - d),
                indexed: listed.contains(name),
                entry_cost: index_entry_cost(&index, name),
                homes: homes_for(&corpus.docs, name, &reached),
                name: name.clone(),
            }
        })
        .collect();
    standings.sort_by(|a, b| a.read.total_cmp(&b.read).then(a.name.cmp(&b.name)));

    report(&corpus, &standings, &listed, half_life, today, &index);
    Ok(())
}

/// Whether this is a rule to be absorbed rather than a fact to be looked up.
fn behavioural(name: &str) -> bool {
    name.starts_with("feedback_") || name.starts_with("user_")
}

fn report(
    corpus: &Corpus,
    standings: &[Standing],
    listed: &BTreeSet<String>,
    half_life: f64,
    today: i64,
    index: &str,
) {
    println!(
        "{} memories, {} in the index ({} bytes), half-life {half_life:.0} days, as of day {today}",
        corpus.docs.len(),
        listed.len(),
        index.len()
    );

    let unmeasured = standings
        .iter()
        .filter(|s| s.read == 0.0 && s.edit == 0.0)
        .count();
    println!(
        "{unmeasured} were neither opened nor changed in any transcript the corpus still holds\n"
    );

    println!("DEMOTION CANDIDATES — indexed, least consulted, and already at home elsewhere");
    println!(
        "  {:<58} {:>7} {:>7} {:>6} {:>5} {:>6}  home",
        "memory", "opens", "halved", "edits", "bytes", "maybe"
    );
    let picked: Vec<&Standing> = standings
        .iter()
        .filter(|s| s.indexed && !behavioural(&s.name) && s.read <= 1.0 && !s.homes.is_empty())
        .take(25)
        .collect();

    // ⚠ **The set, not the sum.** Each `home` above was found against the index
    // as it stands, and that index still carries every other candidate's line.
    // So a pair that links only each other is each other's home and both read as
    // housed — until both lines go together. Asking the invariant once, of the
    // whole set, is the only form of this question that has the right answer.
    let names: BTreeSet<String> = picked.iter().map(|s| s.name.clone()).collect();
    let after = reachable_without(&corpus.docs, index, &names);

    let mut recovered = 0usize;
    let mut strands: Vec<&Standing> = Vec::new();
    for s in &picked {
        let safe = after.contains(&s.name);
        if safe {
            recovered += s.entry_cost;
        } else {
            strands.push(s);
        }
        let home = s.homes.first().map(String::as_str).unwrap_or("—");
        println!(
            "  {:<58} {:>7.2} {:>7.2} {:>6.2} {:>5} {:>6}  {home}{}",
            s.name,
            s.read,
            s.read_halved,
            s.edit,
            s.entry_cost,
            s.maybe_reads,
            if safe { "" } else { "   ⚠ STRANDS" }
        );
    }
    println!(
        "  → {recovered} bytes if the {} safe ones were demoted TOGETHER",
        picked.len() - strands.len()
    );
    if !strands.is_empty() {
        println!(
            "  ⚠ {} of these are housed only by another candidate — demote them and\n\
             \x20    nothing reaches them. Give each a home outside this set first.",
            strands.len()
        );
        for s in &strands {
            println!("       {} — housed only by {:?}", s.name, s.homes);
        }
    }
    println!();

    println!("NO HOME — indexed and rarely opened, but nothing live links them.");
    println!("  Demoting one of these strands it. Give it a home first, or leave it listed.");
    for s in standings
        .iter()
        .filter(|s| {
            s.indexed && s.homes.is_empty() && s.read <= 1.0 && !behavioural(s.name.as_str())
        })
        .take(15)
    {
        println!("  {:<58} {:>7.2}", s.name, s.read);
    }
    println!();

    println!("DEMOTED BUT STILL CONSULTED — reached without the index carrying them.");
    println!("  The ratchet's counter-evidence: these were not silenced by being unlisted.");
    let mut live_archive: Vec<&Standing> = standings
        .iter()
        .filter(|s| !s.indexed && s.read > 0.5)
        .collect();
    live_archive.sort_by(|a, b| b.read.total_cmp(&a.read));
    for s in live_archive.iter().take(15) {
        let last = s
            .last_open
            .map_or("never".to_string(), |d| format!("{d}d ago"));
        println!("  {:<58} {:>7.2}  last {last}", s.name, s.read);
    }
    println!();

    println!("RULES — reported apart, never ranked against the rest.");
    println!("  For these the index LINE is the memory: it is followed from the teaser and");
    println!("  the file is never opened, so a low count here is evidence of working well.");
    let mut rules: Vec<&Standing> = standings
        .iter()
        .filter(|s| behavioural(&s.name) && s.indexed)
        .collect();
    rules.sort_by(|a, b| a.read.total_cmp(&b.read));
    let never = rules.iter().filter(|s| s.read == 0.0).count();
    println!(
        "  {} indexed rules, {never} of them never opened in the window\n",
        rules.len()
    );

    // The stability check, stated rather than assumed: if the ordering moves
    // when the half-life halves, the constant is deciding and not the data.
    let by_trusted: Vec<&str> = standings.iter().map(|s| s.name.as_str()).take(30).collect();
    let mut halved: Vec<&Standing> = standings.iter().collect();
    halved.sort_by(|a, b| {
        a.read_halved
            .total_cmp(&b.read_halved)
            .then(a.name.cmp(&b.name))
    });
    let by_halved: Vec<&str> = halved.iter().map(|s| s.name.as_str()).take(30).collect();
    let moved = by_trusted.iter().filter(|n| !by_halved.contains(n)).count();
    println!("STABILITY: {moved} of the 30 least-consulted change when the half-life is halved.");
    println!("  A large number here means the constant is deciding rather than the history.");
}
