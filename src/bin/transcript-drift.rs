//! Does a transcript's already-read prefix stay put? Measured, not assumed.
//!
//!     cargo run --release --bin transcript-drift          # record, or compare
//!
//! ⚠ **This is the evidence a resumable mine rests on.** Making the mine cheap
//! means reading only what was appended, and that is sound only if the CLI
//! appends. It reads like it does not: the CLI writes earlier stretches of a
//! conversation back into the same file. But those copies are appended, a median
//! 21 MB apart, and the prefix does not move
//! (`reference_claude_transcript_rewrites_history`, memview#1240).
//!
//! ⚠ **A wrong resume is SILENT.** It mines from an offset that means something
//! else and reports no error, so the claim has to keep being checked rather than
//! established once. Two files watched for 25 minutes is where this started;
//! this makes it the whole corpus, repeatably, and every run adds a longer
//! window than the last.
//!
//! First run records. Every later run compares, reports what drifted, and
//! re-records — so a `Rewritten` here is the finding that would have made a
//! resumed mine wrong.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use reader::watermark::{Drift, Watermark, drift, observe};

fn main() -> Result<()> {
    let at = reader::home::cache("transcript-drift.json");
    // ⚠ **An absent file and an unreadable one are different answers.** Absent
    // is a legitimate first run. Unreadable is a lost baseline, and defaulting
    // it to empty would print "first run: recorded 1156" and re-record — the
    // comparison silently gone, in the one tool whose whole job is to notice
    // that something changed underneath.
    let previous: BTreeMap<String, Watermark> = match std::fs::read_to_string(&at) {
        Ok(text) => serde_json::from_str(&text).with_context(|| {
            format!(
                "{} exists but is not readable as watermarks — delete it to start over",
                at.display()
            )
        })?,
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
        Err(why) => return Err(why).with_context(|| format!("reading {}", at.display())),
    };

    let mut now: BTreeMap<String, Watermark> = BTreeMap::new();
    let (mut unchanged, mut grew, mut appended_bytes) = (0usize, 0usize, 0u64);
    let mut rewritten: Vec<String> = Vec::new();
    let mut shrank: Vec<String> = Vec::new();
    let mut fresh = 0usize;

    for path in memview::blame::transcripts(&reader::home::projects_dir()) {
        let key = path.to_string_lossy().into_owned();
        match previous.get(&key) {
            None => fresh += 1,
            Some(mark) => match drift(&path, mark) {
                Drift::Unchanged => unchanged += 1,
                Drift::Grew { by } => {
                    grew += 1;
                    appended_bytes += by;
                }
                // ⚠ These two are the whole point of the tool. Either one means
                // a resumed read would have started in the wrong place.
                Drift::Rewritten => rewritten.push(key.clone()),
                Drift::Shrank => shrank.push(key.clone()),
                Drift::Unknown => {}
            },
        }
        if let Some(mark) = observe(&path) {
            now.insert(key, mark);
        }
    }

    // ⚠ **What a resumable mine WOULD do tonight, printed beside the drift it is
    // built on.** The guard is all-or-nothing — one unresumable file discards
    // every carried artefact — so the question that decides whether resuming is
    // worth anything is how often that fires in practice, and the only honest
    // way to answer it is against this corpus, nightly, over time.
    let marks: BTreeMap<String, reader::watermark::Resume> = previous
        .iter()
        .map(|(k, m)| (k.clone(), reader::watermark::Resume::fresh(m.clone())))
        .collect();
    let files: Vec<std::path::PathBuf> = memview::blame::transcripts(&reader::home::projects_dir());
    if !marks.is_empty() {
        match reader::watermark::plan(&marks, &files) {
            reader::watermark::Plan::Full { because } => {
                println!("a mine could NOT resume tonight: {because}");
            }
            reader::watermark::Plan::Resume { tails, whole, gone } => {
                println!(
                    "a mine could resume: {} tail(s), {} new, {} gone — {} of {} files untouched",
                    tails.len(),
                    whole.len(),
                    gone.len(),
                    files.len() - tails.len() - whole.len(),
                    files.len()
                );
            }
        }
    }

    if previous.is_empty() {
        println!("first run: recorded {} transcripts", now.len());
    } else {
        println!(
            "{} unchanged, {grew} grew (+{:.1} MB), {fresh} new",
            unchanged,
            appended_bytes as f64 / 1_048_576.0
        );
        if rewritten.is_empty() && shrank.is_empty() {
            println!("no prefix moved — every file that changed did so by appending");
        }
        for one in &rewritten {
            println!("  ⚠ REWRITTEN  {one}");
        }
        for one in &shrank {
            println!("  ⚠ SHRANK     {one}");
        }
        if !rewritten.is_empty() || !shrank.is_empty() {
            println!(
                "\n{} file(s) could not have been resumed. A resumable mine must re-read\n\
                 these whole, which `Drift::resumable()` already refuses to skip.",
                rewritten.len() + shrank.len()
            );
        }
    }

    memview::atomic::write(&at, serde_json::to_string_pretty(&now)?.as_bytes())
        .with_context(|| format!("writing {}", at.display()))?;
    Ok(())
}
