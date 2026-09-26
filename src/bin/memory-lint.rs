//! Static analysis for the memory corpus.
//!
//!     cargo run --bin memory-lint [-- <corpus dir>]
//!
//! Defaults to the live corpus. Exits non-zero only on ERROR findings; a rule is
//! introduced as a warning, worked to zero, and promoted in `lint.rs`.
use anyhow::Result;
use memview::couse::CoUse;
use memview::lint;
use memview::store::Corpus;

/// How long to let a half-finished write finish before believing it.
///
/// Measured across every memory creation paired with the next `MEMORY.md` edit:
///
///     <=   3s :  1.5%       <=  60s : 68.8%
///     <=  10s : 41.2%       <= 120s : 72.1%
///     <=  30s : 65.0%       <= 600s : 74.5%
///
/// p50 is 13.7 s and 30 s is the knee. Short of the tail on purpose: p90 runs to
/// 22 hours, which is the thing these rules exist to report. The wait lands only
/// on a gate run that was about to fail (memview #915/#927). Applied to
/// `governs-unreciprocated` by analogy, unmeasured.
const SETTLE: std::time::Duration = std::time::Duration::from_secs(30);

use memview::lint::RACY;

/// Re-read once before reporting a racy rule, and believe the second answer. A
/// retry rather than a timestamp heuristic; a real finding survives it. It
/// narrows the window and does not close it: roughly a third of write windows
/// are longer than 30 s. Untested, because it lives in a bin.
fn settle(
    corpus: Corpus,
    dir: &str,
    couse: Option<&CoUse>,
    roles: Option<&serde_json::Value>,
    accepted: Option<&lint::Accepted>,
) -> Result<(Corpus, Vec<lint::Finding>)> {
    let findings = lint::check_judged(&corpus, couse, roles, accepted);
    let racy = findings.iter().any(|f| RACY.contains(&f.rule));
    if !racy {
        return Ok((corpus, findings));
    }
    eprintln!(
        "index disagrees with the files — re-reading in {SETTLE:?} in case a write is in flight"
    );
    std::thread::sleep(SETTLE);
    let corpus = Corpus::load(dir)?;
    let findings = lint::check_judged(&corpus, couse, roles, accepted);
    Ok((corpus, findings))
}

fn main() -> Result<()> {
    // Refuse a flag this tool does not know (memview#1588).
    memview::flags::reject_unknown(&std::env::args().collect::<Vec<_>>(), &[])?;
    let dir = std::env::args().nth(1).unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.claude/projects/-Users-pippijn-Code/memory")
    });
    let corpus = Corpus::load(&dir)?;
    // Optional: the artefact reads gigabytes of transcripts and is absent on any
    // machine but the Mac.
    let couse = std::path::Path::new(&dir)
        .parent()
        .map(|p| p.join("couse.json"))
        .and_then(|p| CoUse::load(&p));
    // Absent is tolerated here and NOT in `memory-rank`: that tool decides what to
    // PROPOSE demoting, this one only reports a gap, and a lint that cannot run on a
    // fresh checkout is a lint nobody runs.
    let roles: Option<serde_json::Value> =
        std::fs::read_to_string(reader::home::file("memory-roles.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok());
    // The one-word tripwires judged and kept (memview#1784). Private, beside the
    // roles record: it names memories.
    let accepted = lint::Accepted::load(&reader::home::file("accepted-labels.json"))?;
    let (corpus, mut findings) = settle(
        corpus,
        &dir,
        couse.as_ref(),
        roles.as_ref(),
        accepted.as_ref(),
    )?;

    // The one pass that leaves the corpus and asks whether what it says is still
    // true. `CODE_ROOT` overrides for a checkout elsewhere.
    let code_root = std::env::var("CODE_ROOT").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/Code")
    });
    findings.extend(lint::check_world(&corpus, std::path::Path::new(&code_root)));

    // The second outside-the-corpus pass, and the only referee for a stated birthday.
    findings.extend(lint::check_created(
        &corpus,
        &reader::home::cache("memory-created.json"),
    ));
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

    // Said, so the accepted set stays visible rather than silent.
    if let Some(accepted) = &accepted {
        let stale = findings
            .iter()
            .filter(|f| f.rule == "stale-acceptance")
            .count();
        println!(
            "\n{} one-word tripwires accepted (accepted-labels.json, memview#1784)",
            accepted.labels.len() - stale
        );
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

    // One machine-readable line, because the nightly must not awk PROSE:
    // `claude-sync.sh` lifts numbers out of this for `mem_check.py`. Emitted
    // whether or not the root is over, or "under the ceiling" and "did not run"
    // would be the same observation (memview#1260).
    if let Some(index) = corpus.index_md.as_deref() {
        let seen = memview::ceiling::cut(index, memview::ceiling::INDEX_CEILING);
        let mut entries = memview::store::index_links(index);
        entries.sort();
        entries.dedup();
        let mut below = memview::store::index_links(seen.dropped);
        below.sort();
        below.dedup();
        // Teaser coverage rides the same line rather than becoming 349 warnings; as a
        // COUNT it is a trend — how much of the corpus is index-eligible (memview#822).
        let with_teaser = corpus
            .docs
            .values()
            .filter(|d| d.meta.teaser.is_some())
            .count();
        println!(
            "\nindex-stamp {{\"bytes\":{},\"ceiling\":{},\"entries\":{},\"unreachable\":{},\"teasers\":{},\"memories\":{}}}",
            index.len(),
            memview::ceiling::INDEX_CEILING,
            entries.len(),
            below.len(),
            with_teaser,
            corpus.docs.len()
        );
    }

    // Only what this session wrote fails the gate; outside a session every error
    // still fails. See [`lint::passed_for_session`].
    let session = std::env::var("CLAUDE_CODE_SESSION_ID").ok();
    // And what this session last WROTE, a different question from what it created
    // (memview#1553). Looked up only when it can change the answer, since
    // refreshing the record costs a mine catch-up.
    let writers = if session.is_some() && !lint::passed(&findings) {
        last_writers()
    } else {
        None
    };
    // An absent record yields `Unrecorded`, keeping the OLD behaviour; treating
    // "could not ask" as "not yours" would be the masking this rule removes.
    let wrote = |memory: &str| match &writers {
        Some((record, dir, me)) => lint::wrote_by(record.of_memory(dir, memory), me),
        None => lint::Wrote::Unrecorded,
    };
    if lint::passed_for_session(&corpus, &findings, session.as_deref(), wrote) {
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

/// The write record, this session's name in it, and where memories live. Every
/// failure here SKIPS LOUDLY and returns `None`, falling back to `originSessionId`
/// alone — never to "nothing is yours".
fn last_writers() -> Option<(memview::last_writer::LastWriter, String, String)> {
    let at = memview::fresh::Where::from_env();
    let (record, roster) = match memview::fresh::last_writer(&at) {
        Ok(both) => both,
        Err(why) => {
            eprintln!(
                "⚠ no write record ({why}) — findings are attributed by `originSessionId` \
                 alone, so damage to another session's memory will NOT fail this run."
            );
            return None;
        }
    };
    // The agent NAME, from the roster: the record names agents, not session ids,
    // and deriving one from the other by hand once made every file read as
    // somebody else's.
    let me = std::env::var("CLAUDE_AGENT_NAME").ok().or_else(|| {
        std::env::var("CLAUDE_CODE_SESSION_ID")
            .ok()
            .and_then(|id| roster.name_of_session(&id).map(str::to_string))
    })?;
    Some((record, at.memory_dir, me))
}
