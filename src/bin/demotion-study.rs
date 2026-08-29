//! #884's estimate: did demoting a memory from the index cost it opens?
//!
//!     cargo run --release --bin demotion-study        # matching diagnostics
//!     cargo run --release --bin demotion-study -- --harvest
//!
//! ⚠ **This refuses to compute the estimate before 2026-09-11**, and that is the
//! feature. The study is pre-registered: design, matching rule and decision rule
//! were all fixed before any post-period data existed. A tool that would print
//! the answer on request invites looking early and then adjusting something —
//! which is the one thing the pre-registration is for. `--harvest` overrides it
//! and says so loudly, so an early look is at least on the record.
//!
//! The pre-period may be read freely: it is the selection variable, it was
//! complete before treatment, and matching cannot be tuned to an outcome nobody
//! has seen.
//!
//! Reads three private files under `~/.claude` — memory NAMES are private and
//! none of them may ever be committed to this public repo.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use memview::agents::{MemoryDays, day_number};
use memview::study::{Role, Subject, by_arm, match_on_pre_opens};

/// t: the day the treated memories left the index.
const T: &str = "2026-08-14";
/// 28 days each side, as pre-registered.
const WINDOW: i64 = 28;
/// The date the post-period completes. Before this, the estimate is not the
/// study's estimate — it is a peek at a half-finished window.
const HARVEST: &str = "2026-09-11";

fn main() -> Result<()> {
    let harvest = std::env::args().any(|a| a == "--harvest");

    let t = day_number(T).context("t is not a date")?;
    let history: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(
        reader::home::file("index-history.json"),
    )?)?;
    let snap = |day: &str| -> Vec<String> {
        history["snapshots"][day]
            .as_array()
            .map(|xs| {
                xs.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let before: std::collections::BTreeSet<String> = snap("2026-08-10").into_iter().collect();
    let after: std::collections::BTreeSet<String> = snap(T).into_iter().collect();
    anyhow::ensure!(
        !before.is_empty() && !after.is_empty(),
        "a snapshot is missing"
    );

    let days: BTreeMap<String, MemoryDays> = serde_json::from_str(&std::fs::read_to_string(
        reader::home::cache("memory-days.json"),
    )?)?;
    let roles: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(
        reader::home::file("memory-roles.json"),
    )?)?;

    // ⚠ Opens are READS. An edit is the author touching their own file, not the
    // corpus being consulted, and counting it would let a memory look consulted
    // because somebody fixed a typo in it.
    let opens = |name: &str, lo: i64, hi: i64| -> u32 {
        days.get(name)
            .map(|d| d.reads.iter().filter(|&&x| lo <= x && x < hi).count())
            .unwrap_or(0) as u32
    };
    let role = |name: &str| match roles["roles"][name].as_str() {
        Some("tripwire") => Some(Role::Tripwire),
        Some("pointer") => Some(Role::Pointer),
        _ => None,
    };

    let mut subjects = Vec::new();
    for (name, treated) in before
        .difference(&after)
        .map(|n| (n, true))
        .chain(after.iter().map(|n| (n, false)))
    {
        subjects.push(Subject {
            pre: opens(name, t - WINDOW, t),
            post: opens(name, t, t + WINDOW),
            name: name.clone(),
            treated,
            role: role(name),
        });
    }

    let matching = match_on_pre_opens(&subjects);
    let treated = subjects.iter().filter(|s| s.treated).count();
    println!("treated {treated}, control {}", subjects.len() - treated);
    println!(
        "matched {} pair(s); {} treated had no control at their pre-period level",
        matching.pairs.len(),
        matching.unmatched.len()
    );
    let informative = matching
        .pairs
        .iter()
        .filter(|p| p.treated.can_fall())
        .count();
    println!(
        "of those pairs, {informative} can move at all — the rest were at zero opens \
         before treatment and cannot fall"
    );

    if !harvest {
        println!(
            "\nthe post-period completes {HARVEST}. Not computing the estimate before then \
             — re-run with --harvest on or after that date."
        );
        return Ok(());
    }
    let today = memview::study::today();
    if today.as_str() < HARVEST {
        println!(
            "\n⚠ {today} is BEFORE {HARVEST}: this is a peek at an unfinished window, \
                  not the study's estimate. Recorded as such."
        );
    }
    println!("\narm                 pairs  informative   treated   control       DiD");
    for (arm, e) in by_arm(&matching) {
        println!(
            "{arm:<18} {:6} {:12} {:9.2} {:9.2} {:9.2}",
            e.pairs, e.informative, e.treated_change, e.control_change, e.did
        );
    }
    Ok(())
}
