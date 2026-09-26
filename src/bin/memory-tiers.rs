//! What the root is made of, what has fallen out of its lease, and the one
//! exchange that would change it (#1210).
//!
//!     cargo run --release --bin memory-tiers
//!     cargo run --release --bin memory-tiers -- --lease-days 21 --breadth 5
//!     cargo run --release --bin memory-tiers -- --excluding memview
//!
//! A REPORT, not an editor, like `memory-rank`. Breadth — distinct agents, not
//! opens — is the measure: volume cannot separate forty reads by one session
//! from a few by many, and a set cardinality survives the transcripts'
//! duplication. Unprovable opens are shown and never scored (#1214), so breadth
//! is a floor. A stale mine is disclosed, never refused: the refusal it replaced
//! forced two full re-mines in an afternoon over one changed file (#1240).
//!
//! #884's freeze runs until [`HARVEST`] and is on the SPLIT; what it covers is
//! printed under HELD. A demotion is proposed only for a memory judged a
//! POINTER through [`memview::study::role_for`] — frontmatter first, the record
//! behind it (memview#1537) — since for a tripwire a low open count is success.
//!
//! Reads four private files under `~/.claude`; memory NAMES are private and
//! none may ever be committed to this public repo.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use memview::agents::{MemoryDays, day_number};
use memview::store::{
    Corpus, homes_for, incoming_links, index_entry_cost, index_links, reachable_without,
};
use memview::study::{role_for, states_a_claim};
use memview::tiers::{
    Entry, Held, HeldEntry, Role, Thresholds, census, expired, median_entry_cost, propose,
};

/// The size the root is truncated at — see [`memview::lint::INDEX_CEILING`].
/// Defined THERE, not here: written twice it was one edit from two ceilings.
const CEILING: usize = memview::lint::INDEX_CEILING;

/// When #884's freeze lifts and the held half of the trade becomes actionable.
const HARVEST: &str = "2026-09-11";

/// How far back a lease crossing still counts as news.
const CROSSED_WITHIN: i64 = 7;

fn main() -> Result<()> {
    // Refuse a flag this tool does not know (memview#1588).
    memview::flags::reject_unknown(
        &std::env::args().collect::<Vec<_>>(),
        &["--breadth", "--lease-days", "--excluding"],
    )?;
    let args: Vec<String> = std::env::args().collect();
    // Both refuse a bad value rather than defaulting past it: a defaulted
    // `--breadth bogus` would produce the default tiering while reading as
    // parameterised.
    let at = Thresholds {
        lease_days: memview::flags::value_of(
            &args,
            "--lease-days",
            Thresholds::default().lease_days,
        )?,
        tenure_breadth: memview::flags::value_of(
            &args,
            "--breadth",
            Thresholds::default().tenure_breadth,
        )?,
        ..Thresholds::default()
    };

    let home = std::env::var("HOME").unwrap_or_default();
    let root = std::env::var("CLAUDE_DIR").unwrap_or_else(|_| format!("{home}/.claude"));
    let memory_dir = std::env::var("MEMORY_DIR")
        .unwrap_or_else(|_| format!("{root}/projects/-Users-pippijn-Code/memory"));

    let corpus = Corpus::load(&memory_dir)?;
    // Brought up to date before it is read: #1210 came within one step of arguing
    // a demotion from breadth figures that were zero only because the mine had not
    // seen the memories. Refreshing costs about 0.3s.
    let mined = memview::fresh::mined(
        &memview::fresh::Where::from_env(),
        memview::agents::Needs::MEMORIES,
    )
    .with_context(|| {
        format!(
            "refreshing {}",
            reader::home::cache("agents.json").display()
        )
    })?;

    // Disclose a stale mine; do NOT refuse on one (#1240). Ages come from
    // `memory-created.json` and `today`, never from the mine, and nothing here is
    // decay-weighted, so a stale mine means one thing: a floor, stated below.
    let projects = std::env::var("PROJECTS_DIR").unwrap_or_else(|_| format!("{root}/projects"));
    let freshness = mined.freshness(
        &[std::path::Path::new(&projects)],
        std::env::var("CLAUDE_CODE_SESSION_ID").ok().as_deref(),
        &home,
    );

    // The wall clock: an age is a fact about the corpus and the calendar.
    let now = memview::couse::stamp(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    );
    let today = day_number(&now).unwrap_or(0);
    let index = corpus.index_md.clone().unwrap_or_default();
    // The line a reader actually meets, by memory. Read once here rather than in
    // `Entry`'s builder: one parsed pass over the index, as `index_entries` says.
    let labels: BTreeMap<String, String> = memview::store::index_entries(&index)
        .into_iter()
        .map(|entry| (entry.name, entry.label))
        .collect();
    let listed: BTreeSet<String> = index_links(&index).into_iter().collect();
    let reached = reachable_without(&corpus.docs, &index, &BTreeSet::new());
    // Built ONCE: per memory it was ~446,000 markdown parses.
    let incoming = incoming_links(&corpus.docs);
    // How far each memory sits from the index, with nothing demoted.
    let depths = memview::store::depths_without(&corpus.docs, &index, &BTreeSet::new());

    let created: BTreeMap<String, serde_json::Value> =
        read_json(&reader::home::cache("memory-created.json"))?;
    let days: BTreeMap<String, MemoryDays> = read_json(&reader::home::cache("memory-days.json"))?;
    let roles: serde_json::Value = read_json(&reader::home::file("memory-roles.json"))?;

    // #884's two arms, which is what the freeze is on.
    let arm = |which: &str| -> BTreeSet<String> {
        roles["arms"][which]
            .as_array()
            .map(|xs| {
                xs.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let frozen: BTreeSet<String> = arm("treated").union(&arm("control")).cloned().collect();

    // Ask what the tiering says without one agent's opens — see [`tiers::breadth`]
    // for why the session running this is the one worth subtracting.
    let excluding = memview::flags::value_of::<String>(&args, "--excluding", String::new())?;
    let excluding = (!excluding.is_empty()).then_some(excluding);
    let entries: Vec<Entry> = corpus
        .docs
        .keys()
        .map(|name| {
            let breadth = memview::tiers::breadth(&mined.agents, name, excluding.as_deref());
            Entry {
                // The memory's own frontmatter first, the sidecar as a fallback: once every
                // memory carries `created:` the sidecar can go (#1240). `get`, not `[]`: a
                // memory no transcript dates has no entry, and a panic would turn a DETECTION
                // gap into a crash.
                created: corpus
                    .docs
                    .get(name)
                    .and_then(|doc| doc.meta.created)
                    .map(|at| at.timestamp() / 86_400)
                    .or_else(|| {
                        created
                            .get(name)
                            .and_then(|v| v["first"].as_str())
                            .and_then(day_number)
                    }),
                breadth: breadth.proven,
                maybe_breadth: breadth.unprovable,
                unswept_breadth: breadth.unswept,
                returns: memview::tiers::returns(&mined.agents, name, excluding.as_deref()),
                last_open: days
                    .get(name)
                    .and_then(|d| d.reads.iter().max())
                    .map(|d| today - d),
                indexed: listed.contains(name),
                entry_cost: index_entry_cost(&index, name),
                // The author's own declaration first, the #884 record behind it.
                role: role_for(
                    corpus.docs.get(name).and_then(|d| d.meta.role.as_deref()),
                    &roles,
                    name.as_str(),
                ),
                claims: labels.get(name).is_some_and(|l| states_a_claim(l)),
                homes: homes_for(&incoming, name, &reached),
                frozen: frozen.contains(name),
                depth: depths.get(name).copied(),
                name: name.clone(),
            }
        })
        .collect();

    report(&corpus, &entries, &index, today, &at);
    if !freshness.unseen.is_empty() {
        println!(
            "\n⚠ the mine is from {} and has not seen {} memory/memories:",
            freshness.generated,
            freshness.unseen.len()
        );
        for name in freshness.unseen.iter().take(10) {
            println!("    {name}");
        }
        println!(
            "  Their breadth reads as 0 because nothing has been mined for them yet — a floor,\n\
             \x20 not a verdict. Ages above are measured from today and are unaffected.\n\
             \x20 Re-mine for their opens:  cargo run --release --bin agents"
        );
    }
    Ok(())
}

/// An absent private file is a stop, not a default: tiering on a missing
/// creation record would put the whole corpus in UNDATED.
fn read_json<T: serde::de::DeserializeOwned + Default>(path: &std::path::Path) -> Result<T> {
    let text = std::fs::read_to_string(path).with_context(|| {
        format!(
            "reading {} — a private artefact under memview's own directory",
            path.display()
        )
    })?;
    Ok(serde_json::from_str(&text)?)
}

fn role_mark(entry: &Entry) -> &'static str {
    match entry.role {
        Some(Role::Tripwire) => "trip",
        Some(Role::Pointer) => "ptr",
        None => "—",
    }
}

fn report(corpus: &Corpus, entries: &[Entry], index: &str, today: i64, at: &Thresholds) {
    let listed = entries.iter().filter(|e| e.indexed).count();
    println!(
        "{} memories, {listed} in the root ({} bytes of {CEILING}), as of day {today}",
        corpus.docs.len(),
        index.len()
    );
    println!(
        "lease {} days, tenure at {}+ distinct agents\n",
        at.lease_days, at.tenure_breadth
    );

    println!("THE ROOT BY TIER — what holds each entry in place, not how good it is");
    let rows = census(entries, today, at);
    let total: usize = rows.values().map(|w| w.bytes).sum();
    for name in ["LEASE", "TENURE", "MIDDLE", "THIN", "UNDATED"] {
        let w = rows.get(name).copied().unwrap_or_default();
        let share = if total == 0 {
            0.0
        } else {
            100.0 * w.bytes as f64 / total as f64
        };
        println!(
            "  {name:<8} {:>4} entries {:>7} b  {share:>5.1}%",
            w.entries, w.bytes
        );
    }
    println!(
        "  {:<8} {:>4} entries {total:>7} b",
        "", // the sum is of index entries, not of the file: headings and prose are not entries
        rows.values().map(|w| w.entries).sum::<usize>()
    );
    println!("  the file is larger than the sum: headings and prose are not entries\n");

    let crossed = expired(entries, today, at, CROSSED_WITHIN);
    println!(
        "LEASES THAT RAN OUT IN THE LAST {CROSSED_WITHIN} DAYS — {} of them",
        crossed.len()
    );
    println!("  A crossing is an event; the backlog of older ones is the census above.");
    println!(
        "  {:<52} {:>5} {:>7} {:>6} {:>5}  role",
        "memory", "tier", "breadth", "maybe", "last"
    );
    for entry in crossed.iter().take(20) {
        let last = entry
            .last_open
            .map_or("never".to_string(), |d| format!("{d}d"));
        println!(
            "  {:<52} {:>5} {:>7} {:>6} {last:>5}  {}",
            entry.name,
            entry.tier(today, at).label(),
            entry.breadth,
            entry.maybe_breadth,
            role_mark(entry),
        );
    }
    println!();

    // The exchange. A root already over the ceiling has no headroom, and the trade
    // must pay its own way.
    let headroom = CEILING.saturating_sub(index.len());
    let strands = |set: &[Entry]| -> Vec<String> {
        // The set, not the sum: a pair that links only each other reads as housed until
        // both lines go together.
        let cut: BTreeSet<String> = set.iter().map(|e| e.name.clone()).collect();
        let after = reachable_without(&corpus.docs, index, &cut);
        set.iter()
            .filter(|e| !after.contains(&e.name))
            .map(|e| e.name.clone())
            .collect()
    };
    let trade = propose(entries, today, at, headroom, &strands);

    println!("THE TRADE — one operation, both halves. Headroom before it: {headroom} bytes.",);
    println!(
        "  An admission is budgeted at the median entry, {} bytes; its real line is not written yet.",
        median_entry_cost(entries)
    );
    println!(
        "\n  ADMIT — found by {}+ agents outside a sweep WITHOUT the root carrying them,",
        at.tenure_breadth
    );
    println!("  the strong direction of evidence: they were found without help.");
    // `hops` is a second question, not a tie-breaker: the same breadth from one hop
    // and from four are different traversal costs. Printed rather than scored (#822).
    for (i, entry) in trade.admit.iter().take(15).enumerate() {
        println!(
            "    {:<52} {:>3} agents {:>3} unswept  {:>3} maybe  {:>4}  {:<4} {}",
            entry.name,
            entry.breadth,
            entry.unswept_breadth,
            entry.maybe_breadth,
            entry
                .depth
                .map(|d| format!("{d}h"))
                .unwrap_or_else(|| "—".into()),
            role_mark(entry),
            if i < trade.affordable { "" } else { "no room" }
        );
    }
    if trade.admit.is_empty() {
        println!("    (nothing outside the root has been found by that many agents)");
    }
    if trade.swept_admissions > 0 {
        // An agent whose only opens were in a sweep audited the corpus (#1735).
        println!(
            "    ⚠ {} more clear the bar only through agents that reached them in a sweep \
             (more than {} memories opened in a day) — counted, never admitted.",
            trade.swept_admissions,
            memview::agents::SWEEP
        );
    }
    if trade.unproven_admissions > 0 {
        // Named rather than admitted: 43.7% of opens arrive through the shell, and
        // breadth counts SESSIONS, the axis shell-heavy reading distorts (#1214).
        println!(
            "    ⚠ {} more would clear the bar if unprovable shell opens counted — shown, never scored.",
            trade.unproven_admissions
        );
    }
    if trade.affordable < trade.admit.len() {
        // The finding, not a footnote: entries earned a slot and the root has nowhere
        // to put them.
        println!(
            "    ⚠ {} of {} have earned a slot and there is no room for them.",
            trade.admit.len() - trade.affordable,
            trade.admit.len()
        );
    }

    // What a demotion COSTS is how far its target falls, not whether it survives.
    // Computed with the WHOLE demotion set struck out (#869).
    let after = memview::store::depths_without(
        &corpus.docs,
        index,
        &trade
            .demote
            .iter()
            .map(|e| e.name.clone())
            .collect::<BTreeSet<String>>(),
    );
    println!("\n  DEMOTE — thin, past its lease, and already linked from somewhere live.");
    println!("  `falls` is where the target lands once EVERY line below has gone.");
    for entry in trade.demote.iter().take(15) {
        let home = entry.homes.first().map(String::as_str).unwrap_or("—");
        let falls = memview::tiers::falls(entry.depth, after.get(&entry.name).copied());
        println!(
            "    {:<52} {:>3} agents  {:>4} b  {falls:<8} {home}",
            entry.name, entry.breadth, entry.entry_cost
        );
    }
    if trade.demote.is_empty() {
        println!("    (nothing — see HELD below, and NO HOME in memory-rank)");
    }

    if !trade.held.is_empty() {
        println!(
            "\n  ⚠ HELD — {} qualify on opens and must not be demoted anyway.",
            trade.held.len()
        );
        for (why, note) in [
            (
                Held::Tripwire,
                "the line IS the memory — demoting one deletes the only place it fires",
            ),
            (
                Held::Claims,
                "judged POINTER, but the line states a claim — the record disagrees with \
                 what a reader meets, and `memory-lint`'s `loud-pointer` names it",
            ),
            (
                Held::WorkingSet,
                "one agent keeps coming back to it: its working set, which breadth cannot see",
            ),
            (
                Held::Unjudged,
                "#884 has not judged these, and unjudged is not pointer",
            ),
            (
                Held::Unproven,
                "thin only because their unprovable shell opens do not count",
            ),
            (
                Held::Frozen,
                "pointers, but in #884's arms — actionable after {HARVEST}",
            ),
        ] {
            let group: Vec<&HeldEntry> = trade.held.iter().filter(|h| h.why == why).collect();
            if group.is_empty() {
                continue;
            }
            println!(
                "\n    {:?} — {} of them: {}",
                why,
                group.len(),
                note.replace("{HARVEST}", HARVEST)
            );
            for held in group.iter().take(10) {
                println!(
                    "      {:<50} {:>3} agents  {:>4} b",
                    held.entry.name, held.entry.breadth, held.entry.entry_cost
                );
            }
            if group.len() > 10 {
                println!("      … and {} more", group.len() - 10);
            }
        }
    }

    let net = trade.net();
    println!(
        "\n  → {} in, {} out, net {}{} bytes; root would be {} of {CEILING}",
        trade.affordable,
        trade.demote.len(),
        if net > 0 { "+" } else { "" },
        net,
        index.len() as i64 + net
    );
    if net > 0 {
        println!("  ⚠ this exchange GROWS the root — admit fewer, or demote more first.");
    }
}
