//! Static analysis for the memory corpus.
//!
//!     cargo run --bin memory-lint [-- <corpus dir>]
//!
//! Defaults to the live corpus. Exits non-zero only on ERROR findings, so a
//! rule can be introduced as a warning, worked down to zero, and then promoted
//! in `lint.rs` — after which the corpus can never regress on it.
use anyhow::Result;
use memview::couse::CoUse;
use memview::lint;
use memview::store::Corpus;

/// How long to let a half-finished write finish before believing it.
const SETTLE: std::time::Duration = std::time::Duration::from_secs(3);

/// Rules that a corpus caught mid-write can fail through no fault of its own.
///
/// Writing a memory is two edits — the file, then its line in MEMORY.md — and
/// anything that loads the corpus between them sees a memory that exists and is
/// not indexed. Both rules are ERRORS, and the pre-commit gate runs this, so a
/// concurrent session writing a memory could fail an unrelated commit for a
/// reason that was never the committer's and that evaporates on retry.
/// `governs-unreciprocated` joined them 2026-08-15 on the same reasoning: a new
/// rule is written before the work it binds is edited to name it, so a corpus
/// read between those two writes sees a violation that fixes itself.
const RACY: [&str; 3] = [
    "not-in-index",
    "index-points-nowhere",
    "governs-unreciprocated",
];

/// Re-read once before reporting a racy rule, and believe the second answer.
///
/// A retry rather than a timestamp heuristic: "was this file written recently"
/// needs a threshold that is wrong in both directions, whereas re-reading asks
/// the corpus the same question again and takes the answer. A real finding
/// survives it — the index is not going to fix itself — so nothing is hidden,
/// and this costs a few seconds only on the runs that were about to fail.
fn settle(
    corpus: Corpus,
    dir: &str,
    couse: Option<&CoUse>,
) -> Result<(Corpus, Vec<lint::Finding>)> {
    let findings = lint::check(&corpus, couse);
    let racy = findings.iter().any(|f| RACY.contains(&f.rule));
    if !racy {
        return Ok((corpus, findings));
    }
    eprintln!(
        "index disagrees with the files — re-reading in {SETTLE:?} in case a write is in flight"
    );
    std::thread::sleep(SETTLE);
    let corpus = Corpus::load(dir)?;
    let findings = lint::check(&corpus, couse);
    Ok((corpus, findings))
}

fn main() -> Result<()> {
    let dir = std::env::args().nth(1).unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.claude/projects/-Users-pippijn-Code/memory")
    });
    let corpus = Corpus::load(&dir)?;
    // Optional: the artefact is produced by `cargo run --bin couse`, which reads
    // gigabytes of transcripts. Absent on any machine but the Mac, and the lint
    // is still worth running without it.
    let couse = std::path::Path::new(&dir)
        .parent()
        .map(|p| p.join("couse.json"))
        .and_then(|p| CoUse::load(&p));
    let (corpus, mut findings) = settle(corpus, &dir, couse.as_ref())?;

    // The one pass that leaves the corpus and asks whether what it says is still
    // true. `CODE_ROOT` overrides for a checkout somewhere else; the default is
    // the tree every memory writes paths against.
    let code_root = std::env::var("CODE_ROOT").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/Code")
    });
    findings.extend(lint::check_world(&corpus, std::path::Path::new(&code_root)));
    findings.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then(a.rule.cmp(b.rule))
            .then(a.memory.cmp(&b.memory))
    });

    let reasons = lint::rule_reasons();

    println!("{} memories in {dir}\n", corpus.docs.len());

    let mut current = "";
    for finding in &findings {
        if finding.rule != current {
            current = finding.rule;
            let (sev, why) = reasons
                .get(finding.rule)
                .copied()
                .unwrap_or((lint::Severity::Warning, ""));
            println!("\n{}  {}\n    {why}", sev.label(), finding.rule);
        }
        println!("    {:<58} {}", finding.memory, finding.detail);
    }

    println!("\nrelations in use:");
    for (relation, count) in lint::relation_usage(&corpus) {
        println!("    {relation:<14} {count}");
    }

    let tally = lint::tally(&findings);
    println!("\n{} findings", findings.len());
    for (rule, count) in &tally {
        let sev = reasons
            .get(rule)
            .map(|(s, _)| s.label())
            .unwrap_or("warning");
        println!("    {sev:<8} {rule:<22} {count}");
    }

    if lint::passed(&findings) {
        println!("\nno errors");
        Ok(())
    } else {
        std::process::exit(1);
    }
}
