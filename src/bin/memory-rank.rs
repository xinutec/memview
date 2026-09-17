//! Which memories the index is still earning its size on, from history.
//!
//!     cargo run --release --bin memory-rank [-- --half-life 7]
//!
//! A REPORT, and deliberately not an editor: what is live is a judgement, and
//! the cut stays Pippijn's (`feedback_memory_index_is_the_working_set`).
//!
//! ## What is counted
//!
//! Days, not opens; only `Read` and `Edit` tool calls and shell reads (mentions
//! are unusable — re-injection made one name recur 3,370 times). A corpus grep
//! that MATCHED is shown beside a row and never scored: ~17% of the corpus
//! arrives only that way (memview#1238), but 8 agents run corpus-wide greps,
//! so scoring it would compress the bottom of the list where demotions are
//! decided. `maybe_reads` — shell reads whose success cannot be established — are
//! discarded (#1214).
//!
//! ## The two hazards it prints rather than hides
//!
//! The teaser paradox: for the best entries the index LINE is the memory, so
//! opens under-measure exactly the rules doing their job. Which entries those
//! are is a JUDGEMENT through [`memview::study::role_for`], never a name prefix
//! — the prefix test put 192 tripwires on the demotion list (memview#884).
//!
//! The ratchet: being listed causes opens, so the measurement is entangled with
//! the intervention. DEMOTED BUT STILL CONSULTED is the counter-evidence.

use std::collections::BTreeSet;

use anyhow::{Context, Result};
use memview::agents::{HALF_LIFE_DAYS, day_number, weighted};
use memview::store::{
    Corpus, homes_for, incoming_links, index_entry_cost, index_links, reachable_without,
};
use memview::study::{Role, role_for};

/// How a memory stands: what it cost, what it was used for, and whether the
/// index is what is holding it up.
struct Standing {
    name: String,
    /// Weighted days it was opened, at the trusted half-life and at half of it.
    read: f64,
    read_halved: f64,
    /// Weighted days it was changed, apart from opens: a memory nobody reads but
    /// somebody keeps correct is an archive entry in good standing.
    edit: f64,
    /// Opens the miner could not prove happened. Shown, never scored (#1214):
    /// counting them overstates the record, discounting them invents a factor.
    maybe_reads: usize,
    /// Times a corpus-wide search printed a LINE of this memory back. Shown, never
    /// scored, for a different reason: 8 agents run corpus-wide greps, so scoring it
    /// would lift the least-read memories most (memview#1238).
    grep_matches: usize,
    /// Days since it was last opened at all, or `None` if never.
    last_open: Option<i64>,
    /// Whether `MEMORY.md` links it directly.
    indexed: bool,
    /// Bytes its index line spends, which is what a demotion actually recovers.
    entry_cost: usize,
    /// Reachable memories that already link it — the homes a demotion could land
    /// in without stranding it.
    homes: Vec<String>,
    /// What the index line is FOR. `None` is unexamined, not "safe to demote" —
    /// see [`memview::study::role_for`].
    role: Option<Role>,
}

fn main() -> Result<()> {
    // Refuse a flag this tool does not know (memview#1588).
    memview::flags::reject_unknown(&std::env::args().collect::<Vec<_>>(), &["--half-life"])?;
    let args: Vec<String> = std::env::args().collect();
    // Refuses a bad value rather than defaulting past it: `--half-life bogus` once
    // printed the default ranking wearing a parameter's name.
    let half_life = memview::flags::value_of(&args, "--half-life", HALF_LIFE_DAYS)?;

    let home = std::env::var("HOME").unwrap_or_default();
    let memory_dir = std::env::var("MEMORY_DIR")
        .unwrap_or_else(|_| format!("{home}/.claude/projects/-Users-pippijn-Code/memory"));
    let artefact = std::env::var("AGENTS_FILE").unwrap_or_else(|_| {
        reader::home::cache("agents.json")
            .to_string_lossy()
            .into_owned()
    });

    // Loaded with `?`: a missing judgement is an ERROR, never a report that
    // proposes everything — the prefix test this replaces was wrong for 192 entries.
    let roles: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(reader::home::file("memory-roles.json"))
            .with_context(|| "reading memory-roles.json — the tripwire/pointer judgement")?,
    )?;

    let corpus = Corpus::load(&memory_dir)?;
    // Brought up to date before it is read: catching up now costs about 9 seconds
    // — see `memview::fresh`.
    let mined = memview::fresh::mined(
        &memview::fresh::Where::from_env(),
        memview::agents::Needs::MEMORIES,
    )
    .with_context(|| format!("refreshing {artefact}"))?;

    // Beside the roster: `/api/agents` must not carry this.
    let days_file = std::path::Path::new(&artefact).with_file_name("memory-days.json");
    let memory_days: std::collections::BTreeMap<String, memview::agents::MemoryDays> =
        serde_json::from_str(&std::fs::read_to_string(&days_file).with_context(|| {
            format!(
                "reading {} — mine it with: cargo run --release --bin agents",
                days_file.display()
            )
        })?)?;

    // No staleness check any more: every figure is anchored to the mine's stamp,
    // and a stale artefact moved the day every age is measured from (#1210). The
    // artefact is refreshed above instead, so `--stale-ok` never trains anybody.

    // The artefact's own stamp, so re-reading the report never changes it.
    let today = day_number(&mined.generated).unwrap_or(0);
    let index = corpus.index_md.clone().unwrap_or_default();
    let listed: BTreeSet<String> = index_links(&index).into_iter().collect();

    // Everything the index reaches by link — the invariant a demotion must not break.
    let reached = reachable_without(&corpus.docs, &index, &BTreeSet::new());
    // Built ONCE: per memory it was ~446,000 markdown parses.
    let incoming = incoming_links(&corpus.docs);

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
            let grep_matches = mined
                .agents
                .iter()
                .filter_map(|agent| agent.memories.get(name))
                .map(|use_| use_.grep_matches)
                .sum();
            let reads = days.map(|d| d.reads.clone()).unwrap_or_default();
            let edits = days.map(|d| d.edits.clone()).unwrap_or_default();
            Standing {
                read: weighted(reads.iter().copied(), today, half_life),
                read_halved: weighted(reads.iter().copied(), today, half_life / 2.0),
                edit: weighted(edits.iter().copied(), today, half_life),
                maybe_reads,
                grep_matches,
                last_open: reads.iter().max().map(|d| today - d),
                indexed: listed.contains(name),
                entry_cost: index_entry_cost(&index, name),
                homes: homes_for(&incoming, name, &reached),
                // The author's own declaration first, the #884 record behind it.
                role: role_for(corpus.docs[name].meta.role.as_deref(), &roles, name),
                name: name.clone(),
            }
        })
        .collect();
    standings.sort_by(|a, b| a.read.total_cmp(&b.read).then(a.name.cmp(&b.name)));

    report(&corpus, &standings, &listed, half_life, today, &index);
    Ok(())
}

/// Whether this entry may be PROPOSED for demotion — judged a pointer, and
/// nothing else. This replaced a `feedback_`/`user_` prefix test, which put 136
/// `reference_` and 56 `project_` tripwires on the list: a tripwire works by
/// being read and never opened. The harvest came back uninterpretable, so this
/// rests on the argument, as #884 closed recommending.
fn may_demote(role: Option<Role>) -> bool {
    matches!(role, Some(Role::Pointer))
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
    // `grep` is printed beside a row, never added to `opens`.
    let picked: Vec<&Standing> = standings
        .iter()
        .filter(|s| s.indexed && may_demote(s.role) && s.read <= 1.0 && !s.homes.is_empty())
        .take(25)
        .collect();

    // The set, not the sum: a pair that links only each other reads as housed until
    // both lines go together.
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
            "  {:<58} {:>7.2} {:>7.2} {:>6.2} {:>5} {:>6}  {home}{}{}",
            s.name,
            s.read,
            s.read_halved,
            s.edit,
            s.entry_cost,
            s.maybe_reads,
            if s.grep_matches > 0 {
                format!("   grep×{}", s.grep_matches)
            } else {
                String::new()
            },
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
        .filter(|s| s.indexed && s.homes.is_empty() && s.read <= 1.0 && may_demote(s.role))
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
        // A grep match belongs HERE most: a memory reached without the index carrying it.
        let grep = if s.grep_matches > 0 {
            format!("   grep×{}", s.grep_matches)
        } else {
            String::new()
        };
        println!("  {:<58} {:>7.2}  last {last}{grep}", s.name, s.read);
    }
    // Printed whether or not a row above carries one: the sections are top-15
    // slices, and the reach is how ~17% of the corpus arrives.
    let reached: Vec<&Standing> = standings.iter().filter(|s| s.grep_matches > 0).collect();
    let unindexed = reached.iter().filter(|s| !s.indexed).count();
    println!(
        "\n  {} memories were reached by a corpus SEARCH that printed a line of them\
         \n  ({unindexed} of them unindexed). Shown, never scored: 8 agents grep the whole\
         \n  corpus, so ranking on it would lift the least-read memories most.",
        reached.len()
    );
    println!();

    println!("TRIPWIRES — reported apart, never ranked against the rest.");
    println!("  For these the index LINE is the memory: it is followed from the teaser and");
    println!("  the file is never opened, so a low count here is evidence of working well.");
    let mut rules: Vec<&Standing> = standings
        .iter()
        .filter(|s| matches!(s.role, Some(Role::Tripwire)) && s.indexed)
        .collect();
    rules.sort_by(|a, b| a.read.total_cmp(&b.read));
    let never = rules.iter().filter(|s| s.read == 0.0).count();
    println!(
        "  {} indexed tripwires, {never} of them never opened in the window",
        rules.len()
    );
    // The third state, printed rather than dropped: an unjudged entry is in
    // neither half, and the size of what one unblinded pass does not cover is part
    // of reading the report.
    let unjudged = standings
        .iter()
        .filter(|s| s.indexed && s.role.is_none())
        .count();
    println!(
        "  {unjudged} indexed memories carry NO judgement — held, and in neither half above\n"
    );

    // The stability check: if the ordering moves when the half-life halves, the
    // constant is deciding.
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
