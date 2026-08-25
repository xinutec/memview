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
///
/// ⚠ **Measured 2026-08-15, having been a guess since it was written on
/// 2026-07-30 — and the guess was short by a factor of ten.** It was 3 s, which
/// felt like long enough for two consecutive tool calls. Across **612 memory
/// creations** in this
/// machine's transcripts, paired with the next `MEMORY.md` edit in the same
/// session, that covers one window in sixty-six:
///
///     <=   3s :  1.5%       <=  60s : 68.8%
///     <=  10s : 41.2%       <= 120s : 72.1%
///     <=  30s : 65.0%       <= 600s : 74.5%
///
/// p50 is 13.7 s. **30 s is the knee**; past it each step buys single digits.
///
/// ⚠ **Deliberately short of the tail, because the tail is not a race.** p90 runs
/// to 22 hours — a memory unindexed that long is unreachable for a day, which is
/// the thing these rules exist to report. Waiting it out would only make the
/// gate slower at saying nothing.
///
/// ⚠ **The cost lands on a person, which is the argument AGAINST raising it and
/// it loses.** This runs in the pre-commit gate, so the wait is added to somebody
/// already waiting — but only on a run that was about to fail, on a gate that
/// takes minutes anyway, and the alternative is refusing a commit for a write
/// that was never the committer's. `mem_check.py` took the same 30 s for the
/// mirror-image reason: nobody is watching a daily collector at all
/// (xinutec-infra `dc113c0`, memview #915/#927).
///
/// ⚠ **Calibrated for the index pair, and applied to `governs-unreciprocated` by
/// analogy.** That rule's window is a rule file against the work it binds, which
/// was not measured — same two-edit shape, unmeasured size.
const SETTLE: std::time::Duration = std::time::Duration::from_secs(30);

/// Rules that a corpus caught mid-write can fail through no fault of its own.
///
/// Writing a memory is two edits — the file, then its line in MEMORY.md — and
/// anything that loads the corpus between them sees a memory that exists and is
/// not indexed. Both rules are ERRORS, and the pre-commit gate runs this, so a
/// concurrent session writing a memory could fail an unrelated commit for a
/// reason that was never the committer's and that evaporates on retry.
/// `governs-unreciprocated` was a third until 2026-08-25, when the typed-link
/// requirement was retired — see `feedback_typed_memory_links`.
const RACY: [&str; 2] = ["not-in-index", "index-points-nowhere"];

/// Re-read once before reporting a racy rule, and believe the second answer.
///
/// A retry rather than a timestamp heuristic: "was this file written recently"
/// needs a threshold that is wrong in both directions, whereas re-reading asks
/// the corpus the same question again and takes the answer. A real finding
/// survives it — the index is not going to fix itself — so nothing is hidden,
/// and this costs [`SETTLE`] only on the runs that were about to fail.
///
/// ⚠ **It narrows the window; it does not close it.** At the measured p50 of
/// 13.7 s a single 30 s re-read clears most of what it meets, and roughly a
/// third of write windows are longer than that. A finding that survives means
/// "still inconsistent half a minute later", which is worth reading — not
/// "definitely broken", and not "the settle failed".
///
/// ⚠ **Untested, because it lives in a bin.** `mem_check.py` has four tests for
/// the same mechanism with the sleep injected; this has none, and an untested
/// constant is one nobody has a reason to check. Moving it into the library
/// would fix that and is not this change.
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

    // ⚠ **Only what this session wrote fails the gate.** The corpus is shared, so
    // before this the block landed on whoever committed next rather than on the
    // author. Outside a session — the nightly under launchd — `session` is None
    // and every error still fails, which is what keeps the corpus out of its
    // history until it is fixed. See [`lint::passed_for_session`].
    let session = std::env::var("CLAUDE_CODE_SESSION_ID").ok();
    if lint::passed_for_session(&corpus, &findings, session.as_deref()) {
        if lint::passed(&findings) {
            println!("\nno errors");
        } else {
            println!(
                "\nerrors above, but none of them this session's — not failing. \
                 They are reported to fleetwatch by the nightly (mem_check `delivery`), \
                 and they DO stop the corpus being committed until fixed."
            );
        }
        Ok(())
    } else {
        std::process::exit(1);
    }
}
