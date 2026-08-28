//! What the root is made of, what has fallen out of its lease, and the one
//! exchange that would change it (#1210).
//!
//!     cargo run --release --bin memory-tiers
//!     cargo run --release --bin memory-tiers -- --lease-days 21 --breadth 5
//!
//! `MEMORY.md` is injected into every session before anything is asked, so its
//! cost is paid constantly. Two kinds of entry earn that differently
//! (`docs/memory.md`): **recent** work, live rather than proven, which the tier
//! is supposed to turn over; and **consolidated** rules, which belong there
//! because by the time they matter nobody knows to go looking.
//!
//! ⚠ **A REPORT, and deliberately not an editor**, the same as `memory-rank`.
//! Which entries are live is a judgement — a memory can be correct, rarely
//! opened, and exactly the thing that must sit in front of somebody every
//! session. And the corpus is not memview's to hand-edit: the tools are built
//! here, the memory session runs them.
//!
//! ⚠ **#884's freeze runs until 2026-09-11.** A prospective study has been
//! running on these index lines since 2026-08-14, and the freeze is on the
//! SPLIT: do not re-promote a treated memory, do not demote a control one.
//! Anything the evidence would offer that the freeze covers is printed under
//! HELD rather than dropped, so the proposal can be read now and acted on after.
//!
//! ## What it counts, and what that misses
//!
//! **Breadth, not opens** — how many distinct agents opened a memory, not how
//! often. Volume alone cannot separate forty reads by one session on one
//! afternoon (a topic being worked, which belongs one hop away under its hub)
//! from a few reads each by many sessions (a rule that has consolidated). It is
//! also a set cardinality, so it survives the duplication in the transcripts
//! where a raw count would double.
//!
//! ⚠ **Unprovable opens are shown and never scored** (#1214), so breadth is a
//! floor. `maybe` beside a row is the agents whose only evidence is a shell read
//! after `&&` or inside a script with one exit status.
//!
//! ⚠ **The teaser paradox is NOT corrected for, and must not be corrected for
//! by prefix.** For the best entries the index line IS the memory — read from
//! the teaser, file never opened — so breadth under-measures them. `memory-rank`
//! holds `feedback_`/`user_` names apart for this reason, and #884's finding is
//! that the prefix is the wrong classifier: `reference_` is mostly tripwires.
//! So this reads the role from `memory-roles.json` where #884 has judged one and
//! prints it beside the row, rather than building a second tool on the classifier
//! the first study exists to replace.
//!
//! Reads four private files under `~/.claude` — memory NAMES are private and
//! none of them may ever be committed to this public repo.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use memview::agents::{Agents, MemoryDays, day_number};
use memview::store::{Corpus, homes_for, index_entry_cost, index_links, reachable_without};
use memview::tiers::{Entry, Role, Thresholds, census, expired, median_entry_cost, propose};

/// The size the root is truncated at, from Claude Code's own warning text —
/// "approaching the 24.4KB read limit". Past it the root is cut from the bottom
/// and nothing says which part went missing.
///
/// ⚠ **The conservative reading of an ambiguous figure.** 24.4 KB is either
/// 24,400 or 24,985 bytes depending on which kilobyte is meant, and the cost of
/// guessing high is a silent truncation nobody can see. Guessing low costs a
/// few hundred bytes of headroom.
const CEILING: usize = 24_400;

/// When #884's freeze lifts and the held half of the trade becomes actionable.
const HARVEST: &str = "2026-09-11";

/// How far back a lease crossing still counts as news.
const CROSSED_WITHIN: i64 = 7;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| -> Option<String> {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let at = Thresholds {
        lease_days: flag("--lease-days")
            .and_then(|n| n.parse().ok())
            .unwrap_or(Thresholds::default().lease_days),
        tenure_breadth: flag("--breadth")
            .and_then(|n| n.parse().ok())
            .unwrap_or(Thresholds::default().tenure_breadth),
        ..Thresholds::default()
    };

    let home = std::env::var("HOME").unwrap_or_default();
    let root = std::env::var("CLAUDE_DIR").unwrap_or_else(|_| format!("{home}/.claude"));
    let memory_dir = std::env::var("MEMORY_DIR")
        .unwrap_or_else(|_| format!("{root}/projects/-Users-pippijn-Code/memory"));

    let corpus = Corpus::load(&memory_dir)?;
    let mined =
        Agents::load(std::path::Path::new(&format!("{root}/agents.json"))).with_context(|| {
            format!("reading {root}/agents.json — mine it with: cargo run --release --bin agents")
        })?;

    // ⚠ **Refuse rather than tier on a mine that has not seen the corpus.**
    // Every age below is measured from `generated`, so a stale artefact does not
    // merely omit recent memories — it moves the day every lease is counted
    // from, and a lease is the one thing here that turns on a date. The same
    // refusal `memory-rank` makes, for the same reason (#1210).
    let projects = std::env::var("PROJECTS_DIR").unwrap_or_else(|_| format!("{root}/projects"));
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
            "\ntiering them would measure every lease from the mine, not from now.\n\
             re-mine:  cargo run --release --bin agents\n\
             or pass --stale-ok to tier anyway, knowing the ages lean old."
        );
        std::process::exit(2);
    }

    let today = day_number(&mined.generated).unwrap_or(0);
    let index = corpus.index_md.clone().unwrap_or_default();
    let listed: BTreeSet<String> = index_links(&index).into_iter().collect();
    let reached = reachable_without(&corpus.docs, &index, &BTreeSet::new());

    let created: BTreeMap<String, serde_json::Value> =
        read_json(&format!("{root}/memory-created.json"))?;
    let days: BTreeMap<String, MemoryDays> = read_json(&format!("{root}/memory-days.json"))?;
    let roles: serde_json::Value = read_json(&format!("{root}/memory-roles.json"))?;

    // #884's two arms, which is what the freeze is on. Held together in the
    // roles file, which `demotion-study` already reads.
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

    let entries: Vec<Entry> = corpus
        .docs
        .keys()
        .map(|name| {
            // Breadth is over agents, so an agent that opened one memory forty
            // times counts once — which is the whole distinction being drawn.
            let uses = mined
                .agents
                .iter()
                .filter_map(|agent| agent.memories.get(name));
            let (mut breadth, mut maybe_breadth) = (0, 0);
            for use_ in uses {
                if use_.reads > 0 {
                    breadth += 1;
                } else if use_.maybe_reads > 0 {
                    maybe_breadth += 1;
                }
            }
            Entry {
                // ⚠ `get`, not `[]`. 27 memories have no recovered creation
                // date, and indexing a map by a missing key panics — turning a
                // known DETECTION gap into a crash on the whole report.
                created: created
                    .get(name)
                    .and_then(|v| v["first"].as_str())
                    .and_then(day_number),
                breadth,
                maybe_breadth,
                last_open: days
                    .get(name)
                    .and_then(|d| d.reads.iter().max())
                    .map(|d| today - d),
                indexed: listed.contains(name),
                entry_cost: index_entry_cost(&index, name),
                role: match roles["roles"][name.as_str()].as_str() {
                    Some("tripwire") => Some(Role::Tripwire),
                    Some("pointer") => Some(Role::Pointer),
                    _ => None,
                },
                homes: homes_for(&corpus.docs, name, &reached),
                frozen: frozen.contains(name),
                name: name.clone(),
            }
        })
        .collect();

    report(&corpus, &entries, &index, today, &at);
    Ok(())
}

/// An absent private file is a stop, not a default. Tiering on a missing
/// creation record would put the whole corpus in UNDATED and read like a
/// finding about the corpus rather than about the machine.
fn read_json<T: serde::de::DeserializeOwned + Default>(path: &str) -> Result<T> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading {path} — a private artefact under ~/.claude"))?;
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

    // The exchange. Headroom is what the ceiling allows before anything moves;
    // a root already over it has none, and the trade must pay its own way.
    let headroom = CEILING.saturating_sub(index.len());
    let strands = |set: &[Entry]| -> Vec<String> {
        // ⚠ The set, not the sum. Each home was found against the index as it
        // stands, which still carries every other candidate's line — so a pair
        // that links only each other reads as housed until both lines go
        // together. Ask the invariant once, of the whole set.
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
        "\n  ADMIT — reached by {}+ agents WITHOUT the root carrying them, which is the",
        at.tenure_breadth
    );
    println!("  strong direction of evidence: they were found without help.");
    for (i, entry) in trade.admit.iter().take(15).enumerate() {
        println!(
            "    {:<52} {:>3} agents  {:>3} maybe  {:<4} {}",
            entry.name,
            entry.breadth,
            entry.maybe_breadth,
            role_mark(entry),
            if i < trade.affordable { "" } else { "no room" }
        );
    }
    if trade.admit.is_empty() {
        println!("    (nothing outside the root has been opened by that many agents)");
    } else if trade.affordable < trade.admit.len() {
        // ⚠ The finding, not a footnote: entries earned a slot and the root has
        // nowhere to put them. That argues for a demotion pass, which is a
        // different conclusion from "nothing qualifies".
        println!(
            "    ⚠ {} of {} have earned a slot and there is no room for them.",
            trade.admit.len() - trade.affordable,
            trade.admit.len()
        );
    }

    println!("\n  DEMOTE — thin, past its lease, and already linked from somewhere live.");
    for entry in trade.demote.iter().take(15) {
        let home = entry.homes.first().map(String::as_str).unwrap_or("—");
        println!(
            "    {:<52} {:>3} agents  {:>4} b  {home}",
            entry.name, entry.breadth, entry.entry_cost
        );
    }
    if trade.demote.is_empty() {
        println!("    (nothing — see HELD below, and NO HOME in memory-rank)");
    }

    if !trade.held.is_empty() {
        println!(
            "\n  ⚠ HELD until {HARVEST} — {} qualify on the evidence and are in #884's arms.",
            trade.held.len()
        );
        println!("  Demoting a control entry perturbs a series that has run since 2026-08-14.");
        for entry in trade.held.iter().take(10) {
            println!("    {:<52} {:>3} agents", entry.name, entry.breadth);
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
