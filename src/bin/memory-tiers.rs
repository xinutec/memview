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
//! ⚠ **A stale mine is disclosed, never refused.** Ages come from
//! `memory-created.json` and the wall clock, and nothing here is decay-weighted
//! — breadth is a set cardinality — so an old artefact means one thing only:
//! memories it has not seen show breadth 0, a floor that is printed and not
//! scored. The refusal this replaced fired on one changed file and forced two
//! ~6-minute re-mines in an afternoon (#1240).
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
//! ⚠ **The teaser paradox decides the demote half, and by ROLE, never by name
//! prefix.** For the best entries the index line IS the memory — read from the
//! teaser, file never opened — so breadth under-measures exactly the rules doing
//! their job. `Tier::Thin` is breadth-derived, so a demotion filter that reads
//! only the tier selects those entries first. `memory-rank` holds them back by a
//! `feedback_`/`user_` prefix test; #884's finding is that the prefix is the
//! wrong classifier, since `reference_` is mostly tripwires.
//!
//! So a demotion is proposed only for a memory `memory-roles.json` judges a
//! POINTER. A tripwire is held because demoting it deletes the only place it
//! fires; an unjudged memory is held because an absent judgement is not a
//! pointer. Dropping the prefix test without putting this in its place is what
//! left the half unguarded, and only #884's freeze stopped it reaching a
//! proposal (#1234).
//!
//! Reads four private files under `~/.claude` — memory NAMES are private and
//! none of them may ever be committed to this public repo.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use memview::agents::{MemoryDays, day_number};
use memview::store::{
    Corpus, homes_for, incoming_links, index_entry_cost, index_links, reachable_without,
};
use memview::tiers::{
    Entry, Held, HeldEntry, Role, Thresholds, census, expired, median_entry_cost, propose,
};

/// The size the root is truncated at — see [`memview::lint::INDEX_CEILING`] for
/// the number and how it was measured.
///
/// ⚠ **Defined THERE and not here, deliberately.** It was written out twice, in
/// this tool and in the lint rule that reports the same overage, which is one
/// edit away from the two disagreeing about where the ceiling is while both
/// sound authoritative. A tool that proposes a trade against one reading of
/// "24.4 KB" and a check that warns above the other would be a corpus with two
/// ceilings — which is exactly the ambiguity that stood until it was measured.
const CEILING: usize = memview::lint::INDEX_CEILING;

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
    // ⚠ **Brought up to date before it is read.** The disclosure below is the
    // floor's mitigation, not a substitute for not having a floor: a memory the
    // mine has not seen has no recorded opens, and #1210 came within one step of
    // arguing a demotion from breadth figures that were zero for that reason
    // alone. Refreshing costs about 0.3s — a reader carries only
    // `mine-resume.json`, skips the git walk it does not read, and never writes.
    // What still reaches the disclosure is genuinely unmined, which is the case
    // it was written for.
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

    // ⚠ **Disclose a stale mine; do NOT refuse on one.** This used to exit 2,
    // and its own comment predicted the cost: "a refusal that fires on a
    // harmless change trains people to pass the override." On 2026-08-28 it
    // trained the session that wrote it, twice in one afternoon, and forced two
    // full ~6-minute re-mines to answer questions about a corpus that had
    // changed by one file (#1240).
    //
    // ⚠ **The distortion it stood in for is fixed at the source instead.** Ages
    // come from `memory-created.json` and `today` below, never from the mine —
    // so measuring them from the mine's stamp made a week-old artefact
    // understate every age by a week, silently. Nothing here is decay-weighted:
    // breadth is a set cardinality with no time in it. So a stale mine now
    // means exactly one thing, and it is a floor rather than a skew — memories
    // it has not seen have no recorded opens, which is stated below and not
    // scored.
    let projects = std::env::var("PROJECTS_DIR").unwrap_or_else(|_| format!("{root}/projects"));
    let freshness = mined.freshness(
        &[std::path::Path::new(&projects)],
        std::env::var("CLAUDE_CODE_SESSION_ID").ok().as_deref(),
        &home,
    );

    // ⚠ **The wall clock, not the mine's stamp.** An age is a fact about the
    // corpus and the calendar; the mine contributes nothing to it.
    let now = memview::couse::stamp(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    );
    let today = day_number(&now).unwrap_or(0);
    let index = corpus.index_md.clone().unwrap_or_default();
    let listed: BTreeSet<String> = index_links(&index).into_iter().collect();
    let reached = reachable_without(&corpus.docs, &index, &BTreeSet::new());
    // ⚠ Built ONCE. Asking it per memory meant a full markdown parse of every
    // document for every target — ~446,000 parses of a few megabytes.
    let incoming = incoming_links(&corpus.docs);
    // How far each memory sits from the index, which is the traversal cost a
    // root line buys down. Taken with nothing demoted, so it is today's graph.
    let depths = memview::store::depths_without(&corpus.docs, &index, &BTreeSet::new());

    let created: BTreeMap<String, serde_json::Value> =
        read_json(&reader::home::cache("memory-created.json"))?;
    let days: BTreeMap<String, MemoryDays> = read_json(&reader::home::cache("memory-days.json"))?;
    let roles: serde_json::Value = read_json(&reader::home::file("memory-roles.json"))?;

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
                // ⚠ **The memory's own frontmatter first, the sidecar only
                // as a fallback.** `memory-dated` writes `created:` into each
                // file, which is the versioned copy; once every memory carries
                // one the sidecar is a cache of what the corpus already says and
                // can go (#1240). Until then a memory written before that pass
                // still needs it.
                //
                // ⚠ `get`, not `[]`. Memories no transcript dates have no entry
                // at all, and indexing a map by a missing key panics — turning a
                // known DETECTION gap into a crash on the whole report.
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

/// An absent private file is a stop, not a default. Tiering on a missing
/// creation record would put the whole corpus in UNDATED and read like a
/// finding about the corpus rather than about the machine.
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
    // ⚠ **`hops` is not a tie-breaker on breadth, it is a second question.** A
    // memory reached by many agents from ONE hop already has a short traversal
    // and a root line buys little; the same breadth from four hops out is a
    // reader going a long way, repeatedly, for something the root does not
    // carry. Printed rather than folded into a score, because nothing has
    // measured which way it should weigh yet (#822).
    for (i, entry) in trade.admit.iter().take(15).enumerate() {
        println!(
            "    {:<52} {:>3} agents  {:>3} maybe  {:>4}  {:<4} {}",
            entry.name,
            entry.breadth,
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
        println!("    (nothing outside the root has been opened by that many agents)");
    }
    if trade.unproven_admissions > 0 {
        // ⚠ Named rather than admitted. 43.7% of opens arrive through the shell,
        // so a bar that ignores the unprovable half is a bar decided partly by
        // what was discarded — and breadth counts SESSIONS, which is the axis
        // shell-heavy reading distorts (#1214).
        println!(
            "    ⚠ {} more would clear the bar if unprovable shell opens counted — shown, never scored.",
            trade.unproven_admissions
        );
    }
    if trade.affordable < trade.admit.len() {
        // ⚠ The finding, not a footnote: entries earned a slot and the root has
        // nowhere to put them. That argues for a demotion pass, which is a
        // different conclusion from "nothing qualifies".
        println!(
            "    ⚠ {} of {} have earned a slot and there is no room for them.",
            trade.admit.len() - trade.affordable,
            trade.admit.len()
        );
    }

    // ⚠ **What a demotion COSTS is how far its target falls, not whether it
    // survives.** `homes` answers the second question — is there anything left
    // linking it — and that is a boolean: safe or stranded. One hop further out
    // and four hops further out are both "safe", and they are not the same
    // trade. Computed with the WHOLE demotion set struck out, for the reason
    // `reachable_without` is: two entries that house each other each look one
    // hop away until both lines go (#869).
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
