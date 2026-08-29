//! Rebuild MEMORY.md's membership over time from the transcripts.
//!
//!     cargo run --release --bin index-history            # what it would write
//!     cargo run --release --bin index-history -- --apply
//!
//! ⚠ **`index-history.json` was called perishable and it is not.** Its own `why`
//! field said *"Claude Code prunes its own old transcripts, so this baseline is
//! perishable"*; memview#1240 measured that nothing holding a conversation has
//! been deleted, and #1247 restored one byte-identical from a month-old snapshot.
//! It was unrecomputable because nobody had written this — an absence of work,
//! not a property of the data. With this, it is a cache that can be VERIFIED
//! rather than a record that has to be trusted.
//!
//! ⚠ **Reports by default, and `--apply` is not routine.** #884's pre-period
//! rests on the `2026-08-10` and `2026-08-14` snapshots and is frozen to a
//! harvest on 2026-09-11; `demotion-study` reads exactly those two. Overwriting
//! them from a re-run is how a pre-registered experiment quietly becomes a
//! post-hoc one. Read the diff, then decide.

use std::collections::BTreeSet;

use anyhow::{Context, Result};
use memview::index_history::{Readings, is_the_index, names_in};

fn main() -> Result<()> {
    let apply = std::env::args().any(|a| a == "--apply");
    let at = reader::home::file("index-history.json");

    let mut readings = Readings::default();
    let mut files = 0usize;
    for path in memview::blame::transcripts(&reader::home::projects_dir()) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if !text.contains("MEMORY.md") {
            continue;
        }
        files += 1;
        // The id of a Read whose subject is the index, so its RESULT can be
        // told from every other tool result on the way past. A result arrives
        // on a later line than its call, which is why this is a set rather than
        // a look-behind.
        let mut wanted: BTreeSet<String> = BTreeSet::new();
        for line in text.lines() {
            let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let stamp = row["timestamp"].as_str().unwrap_or_default().to_string();
            let Some(content) = row["message"]["content"].as_array() else {
                continue;
            };
            for part in content {
                match part["type"].as_str() {
                    Some("tool_use")
                        if part["name"].as_str() == Some("Read")
                            && is_the_index(part["input"]["file_path"].as_str().unwrap_or("")) =>
                    {
                        // ⚠ **A PARTIAL read is not a membership.** `Read` takes
                        // `offset`/`limit`, and sessions use them constantly —
                        // `limit: 1` appears 40 times in this corpus. Counted as
                        // a day's index, a three-line read says the index held
                        // three memories, and because the day's LAST reading
                        // wins it beats a full read taken that morning. First
                        // run: days showing 1, 2 and 3 names beside neighbours
                        // holding 200.
                        let partial =
                            !part["input"]["offset"].is_null() || !part["input"]["limit"].is_null();
                        if let Some(id) = part["id"].as_str()
                            && !partial
                        {
                            wanted.insert(id.to_string());
                        }
                    }
                    Some("tool_result") => {
                        let Some(id) = part["tool_use_id"].as_str() else {
                            continue;
                        };
                        if !wanted.contains(id) {
                            continue;
                        }
                        // The body is a string for a file read; anything else is
                        // an error shape and carries no index.
                        if let Some(body) = part["content"].as_str() {
                            readings.absorb(&stamp, names_in(body));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    let seen = readings.seen;
    let rebuilt = readings.history();
    println!(
        "{files} transcripts mention the index; {seen} readings of it, over {} day(s)",
        rebuilt.len()
    );

    // ⚠ The comparison is the point of the tool, not a courtesy. A rebuild that
    // silently replaced the artefact would answer "can this be recomputed?" with
    // a file rather than with evidence.
    let held: serde_json::Value = match std::fs::read_to_string(&at) {
        Ok(text) => serde_json::from_str(&text)
            .with_context(|| format!("{} is not readable as history", at.display()))?,
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(why) => return Err(why).with_context(|| format!("reading {}", at.display())),
    };
    let old = held["snapshots"].as_object().cloned().unwrap_or_default();

    // ⚠ **The days the artefact already holds are reported IN FULL, and the new
    // ones only counted.** They are the ones #884 rests on, and a `take(n)` over
    // one merged list buried every one of them behind 40 newly-recovered days on
    // the first run — a report that hid exactly what it was written to show.
    let mut agree = 0usize;
    let mut known: Vec<String> = Vec::new();
    let mut fresh = 0usize;
    for (day, names) in &rebuilt {
        match old.get(day).and_then(|v| v.as_array()) {
            None => fresh += 1,
            Some(was) => {
                let before: BTreeSet<String> = was
                    .iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect();
                if &before == names {
                    agree += 1;
                } else {
                    let gained = names.difference(&before).count();
                    let lost = before.difference(names).count();
                    known.push(format!(
                        "  ~ {day}  {} → {} names  (+{gained} / -{lost})",
                        before.len(),
                        names.len()
                    ));
                }
            }
        }
    }
    for day in old.keys() {
        if !rebuilt.contains_key(day) {
            known.push(format!("  - {day}  in the artefact, NO reading recovered"));
        }
    }

    println!(
        "against the artefact's {} day(s): {agree} reproduced exactly, {} differ",
        old.len(),
        known.len()
    );
    for line in &known {
        println!("{line}");
    }
    println!("{fresh} day(s) recovered that the artefact does not hold");

    if !apply {
        println!("\n(nothing written; --apply overwrites, and #884's pre-period is in this file)");
        return Ok(());
    }

    let mut out = held.clone();
    out["what"] = serde_json::json!(
        "MEMORY.md membership over time, recovered from Read results in the transcripts."
    );
    // ⚠ The old `why` asserted the transcripts were being pruned. Rebuilding the
    // file while leaving that in place would carry the refuted premise forward
    // in the one shape a source grep never reaches — a data file.
    out["why"] = serde_json::json!(
        "The pre-period for the demotion study in memview #884. NOT perishable: memview#1240 measured that no transcript holding a conversation has been deleted, and `--bin index-history` rebuilds this from them. Kept in git because #884 is pre-registered on the 2026-08-10 and 2026-08-14 snapshots, not because it cannot be recomputed."
    );
    out["snapshots"] = serde_json::to_value(&rebuilt)?;
    memview::atomic::write(&at, serde_json::to_string_pretty(&out)?.as_bytes())
        .with_context(|| format!("writing {}", at.display()))?;
    println!("\nwrote {}", at.display());
    Ok(())
}
