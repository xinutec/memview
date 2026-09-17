//! #884's estimate: did demoting a memory from the index cost it opens?
//!
//!     cargo run --release --bin demotion-study        # matching diagnostics
//!     cargo run --release --bin demotion-study -- --harvest
//!
//! Refuses to compute the estimate before 2026-09-11, and that is the feature:
//! the study is pre-registered, and a tool that prints the answer on request
//! invites looking early. `--harvest` overrides it loudly. The pre-period may be
//! read freely; it was complete before treatment.
//!
//! Reads three private files under `~/.claude`; memory NAMES are private and
//! none may ever be committed to this public repo.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use memview::agents::{MemoryDays, day_number};
use memview::study::{
    Role, by_arm, correct, event_study, match_on_pre_opens, pair_differences, placebo,
    sign_flip_null, subjects_at,
};

/// t: the day the treated memories left the index.
const T: &str = "2026-08-14";
/// 28 days each side, as pre-registered.
const WINDOW: i64 = 28;
/// The date the post-period completes. Before this, the estimate is a peek at a
/// half-finished window.
const HARVEST: &str = "2026-09-11";
/// Fake treatment days, each a whole window before `t`, so the placebo reads
/// only pre-period data.
const PLACEBO_DAYS: [i64; 3] = [-28, -21, -14];
/// Fixed so a band is re-derivable. Any value would do; that it never changes is
/// the property that matters.
const SEED: u64 = 20_260_831;

fn main() -> Result<()> {
    // Refuse a flag this tool does not know (memview#1588).
    memview::flags::reject_unknown(&std::env::args().collect::<Vec<_>>(), &["--harvest"])?;
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

    // Opens are READS: an edit is the author touching their own file.
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

    let treated_arm: Vec<String> = before.difference(&after).cloned().collect();
    let control_arm: Vec<String> = after.iter().cloned().collect();
    let subjects = subjects_at(&treated_arm, &control_arm, t, WINDOW, &opens, &role);

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

    // Printed BEFORE the estimate and on every run: it reads only days before `t`,
    // and a pre-trend found after the number is published is an excuse.
    println!(
        "\nPLACEBO — the same procedure at fake treatment days, where nothing was demoted.\n\
         A working design returns a DiD inside its own null band here."
    );
    let fake: Vec<i64> = PLACEBO_DAYS.iter().map(|d| t + d).collect();
    let placebos = placebo(
        &treated_arm,
        &control_arm,
        &fake,
        WINDOW,
        &opens,
        &role,
        SEED,
    );
    println!("  fake t        pairs       DiD        95% null band   verdict");
    let mut broken = false;
    for p in &placebos {
        let flags = p.flags_an_effect();
        broken |= flags;
        println!(
            "  t{:+4}d {:12} {:9.3}   [{:+.3}, {:+.3}]   {}",
            p.at - t,
            p.estimate.pairs,
            p.estimate.did,
            p.null.lo,
            p.null.hi,
            if flags { "FLAGS AN EFFECT" } else { "null" }
        );
    }
    if broken {
        println!(
            "\n⚠ THE DESIGN FAILS ITS OWN PLACEBO. The procedure reports an effect on windows\n\
             \x20 where no memory was demoted, so the arms were already diverging and a\n\
             \x20 difference-in-differences cannot separate that from the treatment. The\n\
             \x20 harvest estimate below is NOT interpretable as an effect of demotion."
        );
    }

    // The gap in each period, which is what made the failure legible: a series can
    // tell a step from a slope. The bin at -1 is zero BY CONSTRUCTION.
    const BIN: i64 = 14;
    const LEADS: usize = 6;
    println!("\nTHE GAP BY PERIOD — treated minus control, {BIN}-day bins, one pairing throughout");
    let gaps = event_study(&treated_arm, &control_arm, t, BIN, LEADS, &opens, &role);
    for g in &gaps {
        let note = match g.at {
            -1 => "  anchor: zero by construction, matching is exact here",
            0 => "  <- after",
            _ => "",
        };
        println!(
            "  bin {:+}  {:4} pairs   gap {:+.3}{note}",
            g.at, g.pairs, g.gap
        );
    }

    // Printed WITH its own placebo, never alone — see `study::correct`.
    if let Some(c) = correct(&gaps) {
        println!(
            "\n  a linear pre-trend correction would give {:+.3} \
             (observed {:+.3}, trend predicts {:+.3}, slope {:+.3}/bin)",
            c.effect, c.observed, c.expected, c.trend.slope
        );
        let mut refuted = Vec::new();
        for off in PLACEBO_DAYS {
            let fake = event_study(
                &treated_arm,
                &control_arm,
                t + off,
                BIN,
                LEADS,
                &opens,
                &role,
            );
            if let Some(fc) = correct(&fake) {
                refuted.push(format!("{:+.3}", fc.effect));
            }
        }
        println!(
            "  ⚠ the SAME correction at days when nothing was demoted: {} — it does not\n\
             \x20   return to zero either, so it is a diagnostic and not an estimate.",
            refuted.join(", ")
        );
    }

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
    println!(
        "\narm                 pairs  informative   treated   control       DiD   \
         95% null band   verdict"
    );
    for (arm, e) in by_arm(&matching) {
        // The band is over THIS arm's pairs: a sub-arm of 30 has a wider null than one
        // of 130, and the pooled band would call its noise an effect.
        let null = sign_flip_null(&pair_differences(&arm.pairs(&matching)), 4000, SEED);
        println!(
            "{:<18} {:6} {:12} {:9.2} {:9.2} {:9.2}   [{:+.3}, {:+.3}]   {}",
            arm.label(),
            e.pairs,
            e.informative,
            e.treated_change,
            e.control_change,
            e.did,
            null.lo,
            null.hi,
            if broken {
                "UNINTERPRETABLE"
            } else if null.covers(e.did) {
                "indistinguishable from zero"
            } else {
                "distinguishable"
            }
        );
    }
    Ok(())
}
